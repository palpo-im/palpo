//! Run alone against an empty disposable PostgreSQL database; never installs globals.
use diesel::connection::SimpleConnection;
use diesel::sql_types::{BigInt, Text};
use diesel::{Connection, PgConnection, RunQueryDsl};
use palpo::data;

#[derive(Debug, PartialEq, Eq, diesel::QueryableByName)]
struct Alias {
    #[diesel(sql_type = Text)]
    alias_id: String,
    #[diesel(sql_type = Text)]
    room_id: String,
    #[diesel(sql_type = Text)]
    created_by: String,
    #[diesel(sql_type = BigInt)]
    created_at: i64,
    #[diesel(sql_type = BigInt)]
    generation: i64,
}

fn aliases(db: &mut PgConnection, migrated: bool) -> Vec<Alias> {
    let generation = if migrated { "generation" } else { "0::bigint" };
    diesel::sql_query(format!(
        "SELECT alias_id,room_id,created_by,created_at,{generation} AS generation FROM room_aliases ORDER BY alias_id"
    ))
    .load(db)
    .unwrap()
}

fn assert_mapping_preserved(before: &Alias, after: &Alias) {
    assert_eq!(before.alias_id, after.alias_id);
    assert_eq!(before.room_id, after.room_id);
    assert_eq!(before.created_by, after.created_by);
    assert_eq!(before.created_at, after.created_at);
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn legacy_alias_generations_upgrade_once_and_advance_on_same_value_updates() {
    // Preparing pools is lazy, but provide a runtime for their runtime adapters.
    // This synchronous test thread performs the blocking migration operations.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _entered = runtime.enter();
    let url = std::env::var("PALPO_TEST_DATABASE_URL").expect("empty dedicated database required");
    let parsed = url::Url::parse(&url).unwrap();
    assert!(matches!(
        parsed.host_str(),
        Some("localhost" | "127.0.0.1" | "[::1]")
    ));
    let mut control = PgConnection::establish(&url).unwrap();
    #[derive(diesel::QueryableByName)]
    struct Count {
        #[diesel(sql_type = BigInt)]
        count: i64,
    }
    let count: Count = diesel::sql_query(
        "SELECT count(*) AS count FROM pg_tables WHERE schemaname NOT IN ('pg_catalog','information_schema')",
    )
    .get_result(&mut control)
    .unwrap();
    assert_eq!(count.count, 0, "refuse a nonempty database");
    let config: data::DbConfig = serde_json::from_value(serde_json::json!({
        "url": url, "pool_size": 4, "statement_timeout": 5000
    }))
    .unwrap();
    drop(data::PreparedDatabase::prepare(&config).unwrap());
    assert!(!data::is_initialized());

    // Reconstruct exactly the previous four-column alias table and migration
    // history, without rolling back any unrelated migration or installing pools.
    control
        .transaction::<_, diesel::result::Error, _>(|db| {
            db.batch_execute(include_str!(
                "../../data/migrations/2026-11-16-000000_room_alias_generations/down.sql"
            ))?;
            let removed = diesel::sql_query(
                "DELETE FROM __diesel_schema_migrations WHERE version=$1",
            )
            .bind::<Text, _>("20261116000000")
            .execute(db)?;
            assert_eq!(removed, 1, "remove only the generation migration receipt");
            db.batch_execute(
                "INSERT INTO room_aliases(alias_id,room_id,created_by,created_at) VALUES
                ('#legacy-a:alias-upgrade.example','!a:alias-upgrade.example','@alice:alias-upgrade.example',123456789),
                ('#legacy-b:alias-upgrade.example','!b:alias-upgrade.example','@bob:alias-upgrade.example',123456789)",
            )?;
            Ok(())
        })
        .unwrap();
    let legacy = aliases(&mut control, false);
    assert_eq!(legacy.len(), 2);

    drop(data::PreparedDatabase::prepare(&config).unwrap());
    let upgraded = aliases(&mut control, true);
    assert_eq!(upgraded.len(), legacy.len());
    for (before, after) in legacy.iter().zip(&upgraded) {
        assert_mapping_preserved(before, after);
        assert!(after.generation > 0);
    }
    assert_ne!(upgraded[0].generation, upgraded[1].generation);

    // A normal second startup uses Diesel history and must not reset or
    // reallocate generations for existing mappings.
    drop(data::PreparedDatabase::prepare(&config).unwrap());
    assert_eq!(aliases(&mut control, true), upgraded);
    assert!(!data::is_initialized());

    assert_eq!(
        diesel::sql_query("UPDATE room_aliases SET room_id=room_id WHERE alias_id=$1")
            .bind::<Text, _>(&upgraded[0].alias_id)
            .execute(&mut control)
            .unwrap(),
        1
    );
    let updated = aliases(&mut control, true);
    assert_mapping_preserved(&upgraded[0], &updated[0]);
    assert!(updated[0].generation > upgraded[0].generation);
    assert_eq!(updated[1], upgraded[1]);
}
