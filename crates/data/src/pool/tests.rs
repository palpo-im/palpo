use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer};
use horme::{GetConnection, OptionalExtension, RunQueryDsl};

use super::{AsyncPgConnection, DbConfig, DieselPool, Duration};
use crate::DataError;

fn pool(url: &str) -> DieselPool {
    let config: DbConfig = serde_json::from_value(serde_json::json!({
        "url": url,
        "pool_size": 2,
        "connection_timeout": 2000,
        "statement_timeout": 500,
    }))
    .unwrap();
    DieselPool::new(url, &config, 1, "horme lifecycle test").unwrap()
}

fn postgres_pool() -> DieselPool {
    pool(&std::env::var("PALPO_TEST_DATABASE_URL").expect("set a dedicated test database URL"))
}

#[tokio::test]
async fn handles_and_unpolled_queries_do_not_acquire_connections() {
    let pool = pool("postgres://postgres@127.0.0.1/unused");
    let mut db = pool.db();
    let _other = db.clone();
    let query = diesel::select(1.into_sql::<Integer>()).get_result::<i32>(&mut db);
    drop(query);
    drop(db.transaction::<(), DataError, _>(async |_| Ok(())));
    {
        let mut builder = db.build_transaction().read_only().repeatable_read();
        drop(builder.run::<(), DataError, _>(async |_| Ok(())));
    }
    let status = pool.status();
    assert_eq!((status.size, status.waiting), (0, 0));
}

#[tokio::test]
async fn acquisition_errors_are_not_optional_not_found() {
    let pool = pool("postgres://postgres@127.0.0.1/unused");
    pool.inner.close();
    let result = diesel::select(1.into_sql::<Integer>())
        .get_result::<i32>(&mut pool.db())
        .await
        .optional();
    let error = result.unwrap_err();
    assert!(matches!(error, horme::Error::Acquire(_)));
    assert!(matches!(DataError::from(error), DataError::Pool(_)));
    assert_eq!(pool.status().size, 0);
}

// These tests create their own one-connection pool and need no Palpo schema.
#[tokio::test]
#[ignore = "requires a dedicated PALPO_TEST_DATABASE_URL"]
async fn one_connection_supports_nested_queries_and_concurrent_handles() {
    let pool = postgres_pool();
    let mut jobs = tokio::task::JoinSet::new();
    for _ in 0..16 {
        let mut db = pool.db();
        jobs.spawn(async move {
            // Keep the condition temporary and handle alive throughout the body.
            if let Some(value) = diesel::select(1.into_sql::<Integer>())
                .first::<i32>(&mut db)
                .await
                .optional()
                .unwrap()
            {
                tokio::task::yield_now().await;
                let nested = diesel::select(2.into_sql::<Integer>())
                    .get_result::<i32>(&mut db)
                    .await
                    .unwrap();
                assert_eq!(value + nested, 3);
            }
        });
    }
    tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(result) = jobs.join_next().await {
            result.unwrap();
        }
    })
    .await
    .expect("query condition retained the only connection");
    assert_eq!(pool.status().available, 1);
}

#[tokio::test]
#[ignore = "requires a dedicated PALPO_TEST_DATABASE_URL"]
async fn failed_and_cancelled_queries_return_their_leases() {
    let pool = postgres_pool();
    let error = diesel::sql_query("SELECT * FROM pg_catalog.horme_missing_table")
        .execute(&mut pool.db())
        .await
        .unwrap_err();
    assert!(matches!(error, horme::Error::Query(_)));
    assert_eq!(pool.status().available, 1);

    let mut db = pool.db();
    let job = tokio::spawn(async move {
        diesel::sql_query("SELECT pg_sleep(10)")
            .execute(&mut db)
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while pool.status().available != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    job.abort();
    assert!(job.await.unwrap_err().is_cancelled());
    // Driver cleanup may finish the cancelled SQL before recycling the lease.
    tokio::time::timeout(Duration::from_secs(3), async {
        diesel::select(1.into_sql::<Integer>())
            .get_result::<i32>(&mut pool.db())
            .await
            .unwrap();
    })
    .await
    .expect("cancelled query retained the only connection");
}

async fn transaction_id<P: GetConnection<Connection = AsyncPgConnection>>(
    source: &mut P,
) -> horme::Result<i64> {
    diesel::select(diesel::dsl::sql::<BigInt>("txid_current()"))
        .get_result(source)
        .await
}

#[tokio::test]
#[ignore = "requires a dedicated PALPO_TEST_DATABASE_URL"]
async fn transaction_helpers_share_one_lease_and_rollback_releases_it() {
    let pool = postgres_pool();
    let mut db = pool.db();
    let result = db
        .transaction::<_, DataError, _>(async |conn| {
            let first = transaction_id(conn).await?;
            assert_eq!(first, transaction_id(conn).await?);
            assert!(
                tokio::time::timeout(Duration::from_millis(20), pool.checkout())
                    .await
                    .is_err()
            );
            let nested = horme::transaction(conn, async |nested| {
                assert_eq!(first, transaction_id(nested).await?);
                Err::<(), DataError>(diesel::result::Error::RollbackTransaction.into())
            })
            .await;
            assert!(matches!(
                nested,
                Err(DataError::Diesel(
                    diesel::result::Error::RollbackTransaction
                ))
            ));
            assert_eq!(first, transaction_id(conn).await?);
            Err::<(), DataError>(diesel::result::Error::RollbackTransaction.into())
        })
        .await;
    assert!(matches!(
        result,
        Err(DataError::Diesel(
            diesel::result::Error::RollbackTransaction
        ))
    ));
    assert_eq!(pool.status().available, 1);
    transaction_id(&mut db).await.unwrap();
}

#[tokio::test]
#[ignore = "requires a dedicated PALPO_TEST_DATABASE_URL"]
async fn snapshot_builder_preserves_isolation_and_releases_before_follow_up_queries() {
    use diesel::sql_types::Text;

    let pool = postgres_pool();
    let mut db = pool.db();
    for repeatable in [true, false] {
        let builder = db.build_transaction().read_only();
        let mut builder = if repeatable {
            builder.repeatable_read()
        } else {
            builder.read_committed()
        };
        let settings = builder
            .run::<_, DataError, _>(async |conn| {
                Ok(diesel::select((
                    diesel::dsl::sql::<Text>("current_setting('transaction_isolation')"),
                    diesel::dsl::sql::<Text>("current_setting('transaction_read_only')"),
                ))
                .get_result::<(String, String)>(conn)
                .await?)
            })
            .await
            .unwrap();
        assert_eq!(
            settings.0,
            if repeatable {
                "repeatable read"
            } else {
                "read committed"
            }
        );
        assert_eq!(settings.1, "on");
        // Even the builder itself remains alive while another handle uses the only slot.
        transaction_id(&mut pool.db()).await.unwrap();
        let _builder = builder.read_only();
        assert_eq!(pool.status().available, 1);
    }
}

#[tokio::test]
#[ignore = "requires a dedicated PALPO_TEST_DATABASE_URL"]
async fn cancelling_transaction_releases_locks_without_a_later_checkout() {
    let pool = postgres_pool();
    let observer = postgres_pool();
    let mut db = pool.db();
    let (started, ready) = tokio::sync::oneshot::channel();
    let job = tokio::spawn(async move {
        db.transaction::<(), DataError, _>(async |conn| {
            diesel::sql_query("SELECT pg_advisory_xact_lock(7252664,865)")
                .execute(conn)
                .await?;
            started.send(()).unwrap();
            std::future::pending().await
        })
        .await
    });
    tokio::time::timeout(Duration::from_secs(2), ready)
        .await
        .unwrap()
        .unwrap();
    job.abort();
    assert!(job.await.unwrap_err().is_cancelled());
    assert_eq!(
        pool.status().size,
        0,
        "a cancelled transaction must be removed from the idle pool"
    );
    // Inspect from an independent pool before touching the original pool again.
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if diesel::select(diesel::dsl::sql::<diesel::sql_types::Bool>(
                "pg_try_advisory_xact_lock(7252664,865)",
            ))
            .get_result::<bool>(&mut observer.db())
            .await
            .unwrap()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the cancelled backend retained its transaction lock");
    transaction_id(&mut pool.db()).await.unwrap();
}
