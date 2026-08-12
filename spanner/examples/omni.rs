//! Minimal end-to-end example against a local **Spanner Omni** container.
//!
//! Omni is a full Spanner server in a single container. Unlike the Cloud Spanner
//! emulator it accepts **only multiplexed sessions**, so a client that creates
//! regular sessions fails immediately with:
//!
//! ```text
//! InvalidArgument: Please use multiplexed sessions.
//! Only Multiplexed sessions are supported in this environment.
//! ```
//!
//! This example exercises every code path that multiplexed sessions change:
//!
//! 1. session creation (`CreateSession { multiplexed: true }`)
//! 2. a DML-only read-write transaction  -> precommit token from `ExecuteSql`
//! 3. a batch-DML read-write transaction -> precommit token from `ExecuteBatchDml`
//! 4. a streaming query inside a read-write transaction -> token from `PartialResultSet`
//! 5. a mutation-only read-write transaction -> requires `mutation_key` on `BeginTransaction`
//! 6. a read-only single-use transaction
//!
//! ## Running
//!
//! From the `spanner/` directory:
//!
//! ```sh
//! docker compose -f docker-compose.omni.yml up -d
//! docker compose -f docker-compose.omni.yml wait spanner-omni-init
//!
//! SPANNER_EMULATOR_HOST=localhost:9011 cargo run -p gcloud-spanner --example omni
//! ```
//!
//! Set `SPANNER_OMNI_PORT` on both commands to publish somewhere other than 9011.
//!
//! Omni serves a single implicit instance and ignores the project/instance
//! segments of the database path on the data plane, so `DATABASE` below can keep
//! the same shape used against real Cloud Spanner.

use std::env;

use gcloud_spanner::client::{ChannelConfig, Client, ClientConfig, Error};
use gcloud_spanner::key::Key;
use gcloud_spanner::mutation::insert_or_update;
use gcloud_spanner::statement::Statement;
use gcloud_spanner::value::CommitTimestamp;

const DATABASE: &str = "projects/local-project/instances/test-instance/databases/local-database";
/// Matches the default `SPANNER_OMNI_PORT` in docker-compose.omni.yml.
const DEFAULT_HOST: &str = "localhost:9011";

#[tokio::main]
async fn main() -> Result<(), Error> {
    let filter =
        tracing_subscriber::filter::EnvFilter::from_default_env().add_directive("gcloud_spanner=info".parse().unwrap());
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();

    if env::var("SPANNER_EMULATOR_HOST").is_err() {
        env::set_var("SPANNER_EMULATOR_HOST", DEFAULT_HOST);
    }
    println!("connecting to {}", env::var("SPANNER_EMULATOR_HOST").unwrap());

    // `ClientConfig::default()` reads SPANNER_EMULATOR_HOST and skips auth.
    let mut config = ClientConfig {
        channel_config: ChannelConfig {
            num_channels: 1,
            ..Default::default()
        },
        ..Default::default()
    };
    config.session_config.min_opened = 1;
    config.session_config.max_opened = 4;
    // Omni serves nothing else. Real Cloud Spanner and the Cloud Spanner emulator both
    // still accept regular sessions, so this stays opt-in.
    config.session_config.multiplexed = true;

    // Step 1: session creation. Against Omni this is where an unmodified client dies.
    let client = Client::new(DATABASE, config).await?;
    println!("[1/6] client created");

    let guild_id = "omni-guild";
    let owner_id = "omni-owner";

    // Step 2: DML-only read-write transaction.
    // The precommit token arrives on the ExecuteSql response.
    client
        .read_write_transaction::<(), Error, _>(|tx| {
            Box::pin(async move {
                let mut stmt = Statement::new(
                    "INSERT OR UPDATE INTO Guild (GuildId, OwnerUserId, UpdatedAt) \
                     VALUES (@GuildId, @OwnerUserId, PENDING_COMMIT_TIMESTAMP())",
                );
                stmt.add_param("GuildId", &"omni-guild-dml");
                stmt.add_param("OwnerUserId", &owner_id);
                tx.update(stmt).await?;
                Ok(())
            })
        })
        .await?;
    println!("[2/6] DML-only transaction committed");

    // Step 3: batch DML read-write transaction.
    // The precommit token arrives on the ExecuteBatchDml response.
    client
        .read_write_transaction::<(), Error, _>(|tx| {
            Box::pin(async move {
                let stmts = (0..2)
                    .map(|i| {
                        let mut stmt = Statement::new(
                            "INSERT OR UPDATE INTO Guild (GuildId, OwnerUserId, UpdatedAt) \
                             VALUES (@GuildId, @OwnerUserId, PENDING_COMMIT_TIMESTAMP())",
                        );
                        stmt.add_param("GuildId", &format!("omni-guild-batch-{i}"));
                        stmt.add_param("OwnerUserId", &owner_id);
                        stmt
                    })
                    .collect();
                tx.batch_update(stmts).await?;
                Ok(())
            })
        })
        .await?;
    println!("[3/6] batch-DML transaction committed");

    // Step 4: streaming query inside a read-write transaction, then a mutation.
    // Tokens ride on PartialResultSet here.
    let (_, found) = client
        .read_write_transaction::<usize, Error, _>(|tx| {
            Box::pin(async move {
                let mut stmt = Statement::new("SELECT GuildId FROM Guild WHERE OwnerUserId = @OwnerUserId");
                stmt.add_param("OwnerUserId", &owner_id);
                let mut iter = tx.query(stmt).await?;
                let mut ids = vec![];
                while let Some(row) = iter.next().await? {
                    ids.push(row.column_by_name::<String>("GuildId")?);
                }
                drop(iter);
                tx.buffer_write(vec![insert_or_update(
                    "Guild",
                    &["GuildId", "OwnerUserId", "UpdatedAt"],
                    &[&"omni-guild-after-query", &owner_id, &CommitTimestamp::new()],
                )]);
                Ok(ids.len())
            })
        })
        .await?;
    println!("[4/6] streaming query inside read-write transaction saw {found} rows, then committed a mutation");

    // Step 5: mutation-only read-write transaction. No statement ever runs, so the
    // only precommit token available is the one BeginTransaction returns -- and it
    // only returns one when a `mutation_key` was supplied.
    client
        .read_write_transaction::<(), Error, _>(|tx| {
            Box::pin(async move {
                tx.buffer_write(vec![insert_or_update(
                    "Guild",
                    &["GuildId", "OwnerUserId", "UpdatedAt"],
                    &[&guild_id, &owner_id, &CommitTimestamp::new()],
                )]);
                Ok(())
            })
        })
        .await?;
    println!("[5/6] mutation-only transaction committed");

    // Step 6: read it back with a single-use read-only transaction.
    let mut tx = client.single().await?;
    let row = tx
        .read_row("Guild", &["GuildId", "OwnerUserId"], Key::new(&guild_id))
        .await?;
    let row = row.expect("the row written in step 5 should be readable");
    println!(
        "[6/6] read back GuildId={} OwnerUserId={}",
        row.column_by_name::<String>("GuildId")?,
        row.column_by_name::<String>("OwnerUserId")?
    );

    client.close().await;
    println!("\nOK - all six steps succeeded against Spanner Omni");
    Ok(())
}
