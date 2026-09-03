//! Opt-in integration tests only: use an EMPTY, DEDICATED test database.
//! Run with PALPO_TEST_DATABASE_URL set and `cargo test -p palpo --all-features -- --ignored`.

pub fn init() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        use diesel::{Connection, RunQueryDsl};

        #[derive(diesel::QueryableByName)]
        struct TableCount {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            count: i64,
        }

        let url = std::env::var("PALPO_TEST_DATABASE_URL")
            .expect("set PALPO_TEST_DATABASE_URL to an empty dedicated PostgreSQL test database");
        // Initial production migrations contain destructive DDL. Refuse an existing
        // database even if its URL was accidentally placed in the test variable.
        let mut connection = diesel::PgConnection::establish(&url).unwrap();
        let tables: TableCount = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM pg_catalog.pg_tables WHERE schemaname NOT IN ('pg_catalog', 'information_schema')",
        ).get_result(&mut connection).unwrap();
        assert_eq!(tables.count, 0, "database regression tests require an EMPTY dedicated database");
        drop(connection);
        let config: crate::data::DbConfig = serde_json::from_value(serde_json::json!({
            "url": url, "pool_size": 10, "statement_timeout": 5000
        }))
        .unwrap();
        crate::data::init(&config);
    });
}
