use crate::schema::*;
use diesel::r2d2::{ConnectionManager, Pool};
use diesel::{Insertable, Queryable, SqliteConnection};

pub(crate) type Database = Pool<ConnectionManager<SqliteConnection>>;
pub const MIGRATIONS: diesel_migrations::EmbeddedMigrations =
    diesel_migrations::embed_migrations!("./migrations");

#[derive(Debug, Clone, Insertable, Queryable)]
#[diesel(table_name = status)]
pub struct Status {
    pub(crate) id: i32,
    pub(crate) last_block: i64,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = hooks)]
pub struct NewHook {
    pub url: String,
    pub email: String,
    pub filter_addr: Vec<u8>,
}

#[derive(Debug, Clone, Insertable, Queryable)]
#[diesel(table_name = hooks)]
pub struct Hook {
    pub id: i32,
    pub url: String,
    pub email: String,
    pub filter_addr: Vec<u8>,
}

#[derive(Debug, Clone, Insertable, Queryable)]
#[diesel(table_name = calls)]
pub struct Call {
    pub hook_id: i32,
    pub block_num: i32,
    pub ok: Option<String>,
    pub error: Option<String>,
}
