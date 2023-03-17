// @generated automatically by Diesel CLI.

diesel::table! {
    calls (hook_id, block_num) {
        hook_id -> Integer,
        block_num -> Integer,
        ok -> Nullable<Text>,
        error -> Nullable<Text>,
    }
}

diesel::table! {
    hooks (id) {
        id -> Integer,
        url -> Text,
        email -> Text,
        filter_addr -> Binary,
    }
}

diesel::table! {
    status (id) {
        id -> Integer,
        last_block -> BigInt,
    }
}

diesel::joinable!(calls -> hooks (hook_id));

diesel::allow_tables_to_appear_in_same_query!(
    calls,
    hooks,
    status,
);
