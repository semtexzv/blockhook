CREATE TABLE IF NOT EXISTS status
(
    id         INTEGER PRIMARY KEY NOT NULL,
    last_block BIGINT              NOT NULL
);

CREATE TABLE IF NOT EXISTS hooks
(
    id          INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    url         TEXT    NOT NULL,
    email       TEXT    NOT NULL,
    filter_addr BLOB    NOT NULL
);

CREATE TABLE IF NOT EXISTS calls
(
    hook_id   INTEGER NOT NULL REFERENCES hooks,
    block_num INTEGER NOT NULL,

    ok        TEXT DEFAULT NULL,
    error     TEXT DEFAULT NULL,
    PRIMARY KEY (hook_id, block_num)
)