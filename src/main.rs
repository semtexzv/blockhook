use axum::extract::Path;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use diesel::r2d2::ConnectionManager;
use diesel::upsert::excluded;
use diesel::ExpressionMethods;
use diesel::RunQueryDsl;
use diesel_migrations::MigrationHarness;
use diesel_tokio::{AsyncRunQueryDsl, AsyncSimpleConnection};
use futures::{SinkExt, Stream};
use reqwest::{Client, Error, Response, StatusCode};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::future::{pending, poll_fn};
use std::pin::pin;
use url::Url;
use uuid::Uuid;
use web3::{BatchTransport, DuplexTransport, Web3};

use crate::model::{Call, Status, MIGRATIONS};
use crate::schema::hooks::dsl::hooks;
use futures::future::Either;
use futures::{FutureExt, StreamExt};
use model::Database;
use tokio::sync::mpsc::{channel, unbounded_channel, Sender, UnboundedReceiver, UnboundedSender};
use tracing_subscriber::registry::Data;
use web3::transports::WebSocket;
use web3::types::{Block, BlockId, Transaction, H160, H256};

pub mod model;
pub mod schema;

pub async fn updates<C: BatchTransport + DuplexTransport>(
    client: Web3<C>,
) -> impl Stream<Item = Block<Transaction>> {
    client
        .eth_subscribe()
        .subscribe_new_heads()
        .await
        .unwrap()
        .then(move |v| {
            let cl = client.clone();
            async move {
                cl.eth()
                    .block_with_txs(BlockId::from(v.unwrap().hash.unwrap()))
                    .await
                    .unwrap()
                    .unwrap()
            }
        })
}

fn start_per_addr(db: Database, client: Client) -> UnboundedSender<(Url, Notification)> {
    let (tx, rx) = unbounded_channel();
    tokio::spawn(per_addr_task(db, client, rx));
    tx
}

async fn per_addr_task(
    db: Database,
    client: Client,
    mut rec: UnboundedReceiver<(Url, Notification)>,
) {
    let mut queue: VecDeque<(Url, Notification)> = VecDeque::new();
    let mut running = Either::Left(pending::<Result<Response, reqwest::Error>>());
    let mut restart_since: i64 = 0;

    tokio::select! {
        biased;

        done = &mut running => {
            if let Some((url, notif)) = queue.pop_front() {
                // Start the next job
                running = Either::Right(client.post(url)
                    .json(&notif)
                    .send());
            } else if restart_since > 0 {
                // TODO: Reload the queue from the db
                running = Either::Left(pending());
            } else {
                // Reset the future after we've processed it
                running = Either::Left(pending());

            }
        }

        // We need to process immediately to keep the queuing to a minimum.
        item = rec.recv() => {
            match (item, &running) {
                (Some((url, notif)), Either::Left(_)) => {
                    running = Either::Right(client.post(url)
                        .json(&notif)
                        .send());
                }
                (Some(item), Either::Right(_)) => {
                    if queue.len() < 10 {
                        queue.push_back(item);
                    } else {
                        // TODO: Figure out block
                        restart_since = 1;
                    }
                }
                (None, _) => {
                    // Channel closed
                    return
                }
            }
        }
    };
}

struct HookRuntimeInfo {
    hook: model::Hook,
    tx: Option<UnboundedSender<(Url, Notification)>>,
}

struct Dispatcher {
    db: Database,
    cl: Client,
    addrs: HashMap<H160, HookRuntimeInfo>,
}

impl Dispatcher {
    async fn new(db: Database, cl: Client) -> Self {
        let d = hooks.load_async::<model::Hook>(&db).await.unwrap();

        Self {
            db,
            cl,
            addrs: d
                .into_iter()
                .map(|hook| {
                    (
                        H160::from_slice(&hook.filter_addr),
                        HookRuntimeInfo { hook, tx: None },
                    )
                })
                .collect(),
        }
    }

    async fn process(&mut self, b: Block<Transaction>) {
        let block_num = b.number.unwrap().as_u64() as i64;
        for tx in b.transactions {
            if let Some(f) = tx.from.as_ref() {
                if let Some(filt) = self.addrs.get_mut(f) {
                    filt.tx
                        .get_or_insert_with(|| start_per_addr(self.db.clone(), self.cl.clone()))
                        .send((filt.hook.url.parse().unwrap(), Notification::Tx(tx.clone())))
                        .unwrap();
                }
            }
            if let Some(t) = tx.to.as_ref() {
                if let Some(filt) = self.addrs.get_mut(t) {
                    filt.tx
                        .get_or_insert_with(|| start_per_addr(self.db.clone(), self.cl.clone()))
                        .send((filt.hook.url.parse().unwrap(), Notification::Tx(tx.clone())))
                        .unwrap();
                }
            }
        }

        diesel::insert_into(schema::status::table)
            .values(Status {
                id: 0,
                last_block: b.number.unwrap().as_u64() as i64,
            })
            .on_conflict(schema::status::id)
            .do_update()
            .set(schema::status::last_block.eq(excluded(schema::status::last_block)))
            .execute_async(&self.db)
            .await
            .unwrap();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Event {
    Transaction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HookAttrs {
    url: Url,
    since: u64,
    event: Event,
    chain: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Hook {
    id: Uuid,
    #[serde(flatten)]
    a: HookAttrs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "kind")]
enum Notification {
    Test {},
    Tx(web3::types::Transaction),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cl = Client::new();
    let mut db = Database::new(ConnectionManager::new("./hooks.db")).unwrap();
    db.get()?.run_pending_migrations(MIGRATIONS).unwrap();

    let ww = Web3::new(
        WebSocket::new("wss://mainnet.infura.io/ws/v3/4367869609a04a12bf87028f8fa9edaf")
            .await
            .unwrap(),
    );
    let stream = updates(ww).await;

    let mut stream = pin!(stream);
    let mut dispatch = Dispatcher::new(db, cl).await;

    while let Some(b) = stream.next().await {
        println!("Block: {b:?}");
        dispatch.process(b);
    }

    Ok(())
}
type ErrorBody = (StatusCode, String);

macro_rules! bad_request {
    ($txt:expr) => {{
        return Err((StatusCode::BAD_REQUEST, ($txt).to_string()));
    }};
}

fn ise<E: std::error::Error>(e: E) -> ErrorBody {
    todo!()
}

#[axum::debug_handler]
async fn post_hooks(
    cl: Extension<Client>,
    db: Extension<Database>,
    params: Json<HookAttrs>,
) -> Result<Json<Hook>, ErrorBody> {
    // let notif = cl.post(params.url.clone()).json(&Notification {
    //     kind: NotificationKind::Test,
    // });
    // match notif.send().await {
    //     Ok(_) => {}
    //     Err(_) => bad_request!("URL is not reachable"),
    // }
    // diesel::insert_into(schema::hooks::table)
    //     .values(model::NewHook {
    //         email: "".to_string(),
    //         url: "".to_string(),
    //     })
    //     .execute_async(&db)
    //     .await
    //     .map_err(ise)?;
    todo!()
}

#[axum::debug_handler]
async fn get_hook(
    cl: Extension<Client>,
    db: Extension<Database>,
    id: Path<Uuid>,
) -> Result<Json<Hook>, ErrorBody> {
    todo!()
}

#[axum::debug_handler]
async fn del_hook(
    cl: Extension<Client>,
    db: Extension<Database>,
    id: Path<Uuid>,
) -> Result<Json<Hook>, ErrorBody> {
    todo!()
}

// #[tokio::main]
// async fn main() -> anyhow::Result<()> {
//     tracing_subscriber::fmt::init();
//
//     let cl = reqwest::Client::new();
//     let db = Database::new(ConnectionManager::new("")).unwrap();
//     // build our application with a route
//     let app = Router::new()
//         .route("/hooks", post(post_hooks))
//         .route("/hooks/:id", get(get_hook).delete(del_hook))
//         .layer(Extension(cl))
//         .layer(Extension(db));
//
//     let addr = SocketAddr::from((Ipv4Addr::UNSPECIFIED, 9001));
//     tracing::info!("listening on {}", addr);
//     axum::Server::bind(&addr)
//         .serve(app.into_make_service())
//         .await
//         .unwrap();
//     Ok(())
// }
