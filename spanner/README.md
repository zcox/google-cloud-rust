# google-cloud-spanner

Google Cloud Platform spanner library.

[![crates.io](https://img.shields.io/crates/v/gcloud-spanner.svg)](https://crates.io/crates/gcloud-spanner)

* [About Cloud Spanner](https://cloud.google.com/spanner/)
* [Spanner API Documentation](https://cloud.google.com/spanner/docs)
* [Rust client Documentation](https://docs.rs/google-cloud-spanner/latest)

## Installation

```toml
[dependencies]
google-cloud-spanner = { package="gcloud-spanner", version="1.0.0" }
```

## Quickstart

Create `Client` and call transaction API same as [Google Cloud Go](https://github.com/googleapis/google-cloud-go/tree/main/spanner).

```rust
 use google_cloud_spanner::client::Client;
 use google_cloud_spanner::mutation::insert;
 use google_cloud_spanner::statement::Statement;
 use google_cloud_spanner::value::CommitTimestamp;
 use google_cloud_spanner::client::Error;

 #[tokio::main]
 async fn main() -> Result<(), Error> {

     const DATABASE: &str = "projects/local-project/instances/test-instance/databases/local-database";

     // Create spanner client
     let config = ClientConfig::default().with_auth().await.unwrap();
     let mut client = Client::new(DATABASE, config).await.unwrap();

     // Insert
     let mutation = insert("Guild", &["GuildId", "OwnerUserID", "UpdatedAt"], &[&"guildId", &"ownerId", &CommitTimestamp::new()]);
     let commit_timestamp = client.apply(vec![mutation]).await?;

     // Read with query
     let mut stmt = Statement::new("SELECT GuildId FROM Guild WHERE OwnerUserID = @OwnerUserID");
     stmt.add_param("OwnerUserID",&"ownerId");
     let mut tx = client.single().await?;
     let mut iter = tx.query(stmt).await?;
     while let Some(row) = iter.next().await? {
         let guild_id = row.column_by_name::<String>("GuildId");
     }

     // Remove all the sessions.
     client.close().await;
     Ok(())
 }
```

## Running a local Spanner

Two backends, both provisioned with `local-project` / `test-instance` / `local-database`
and the schema in [`tests/ddl/schema.sql`](tests/ddl/schema.sql) — the database the
integration tests expect. Run these from the `spanner/` directory.

**Cloud Spanner emulator** — lightweight, but serves one read-write transaction at a time
(`Aborted: The emulator only supports one transaction at a time`):

```sh
docker compose up -d
docker compose wait spanner-init          # blocks until the schema is applied
cargo test -p gcloud-spanner --test client_test
```

**Spanner Omni** — a full Spanner server, so concurrent read-write transactions and the
real query planner and limits. Needs multiplexed sessions (see below) and roughly 1.5 GB:

```sh
docker compose -f docker-compose.omni.yml up -d
docker compose -f docker-compose.omni.yml wait spanner-omni-init

SPANNER_EMULATOR_HOST=localhost:9011 SPANNER_MULTIPLEXED_SESSIONS=true \
  cargo test -p gcloud-spanner --test client_test
```

In both cases `docker compose up --wait` is *not* sufficient on its own — it returns as
soon as the init service is running, which is before the schema exists. `compose wait`
blocks on it exiting and propagates its exit code.

The emulator defaults to publishing 9010/9020 and Omni to 9011. Override
`SPANNER_EMULATOR_PORT`, `SPANNER_EMULATOR_REST_PORT` and `SPANNER_OMNI_PORT` to run both
stacks side by side or to dodge a port already in use. The two stacks use separate compose
project names, so they do not interfere with each other.

Note that Omni enforces real Cloud Spanner's limits. Three integration tests bulk-insert
20,000 twenty-column rows in one transaction, which exceeds the 120,000-mutation cap, and
fail there while passing on the emulator. They would fail against real Cloud Spanner too.

## Multiplexed sessions and Spanner Omni

[Spanner Omni](https://cloud.google.com/spanner/docs) — a full Spanner server in a single
container, unlike the Cloud Spanner emulator it serves concurrent read-write transactions —
accepts **only multiplexed sessions**. A client that creates regular sessions is rejected at
startup:

```text
InvalidArgument: Please use multiplexed sessions.
Only Multiplexed sessions are supported in this environment.
```

Multiplexed sessions are off by default, because real Cloud Spanner and the Cloud Spanner
emulator both still accept regular sessions. Turn them on either in code:

```rust
let mut config = ClientConfig::default();
config.session_config.multiplexed = true;
```

or with an environment variable, alongside the `SPANNER_EMULATOR_HOST` this library already
reads, so an application can be pointed at Omni without a code change:

```sh
export SPANNER_EMULATOR_HOST=localhost:9011
export SPANNER_MULTIPLEXED_SESSIONS=true
```

With the flag set, the pool creates sessions with `CreateSession { multiplexed: true }`
instead of `BatchCreateSessions`, read-write transactions track the highest-sequence
precommit token seen on `BeginTransaction`, `ExecuteSql`, `ExecuteBatchDml` and streaming
`PartialResultSet` responses and send it with `Commit`, mutation-only transactions supply a
`mutation_key` so they have a token to send, and a commit that comes back asking to be
retried with a fresh token is retried.

A multiplexed session cannot be deleted and does not idle out, so the pool's sizing,
eviction and health-check logic becomes largely redundant. It is left in place; the only
visible effect is a `failed to delete session` warning when the client is closed.

See [`examples/omni.rs`](examples/omni.rs) for a runnable end-to-end example; bring up a
container for it with [`docker-compose.omni.yml`](docker-compose.omni.yml) as above.

## Related project
* [google-cloud-spanner-derive](../spanner-derive)

## Performance 

Result of the 24 hours Load Test.

| Metrics | This library | [Google Cloud Go](https://github.com/googleapis/google-cloud-go/tree/main/spanner) | 
| -------- | ----------------| ----------------- |
| RPS | 438.4 | 443.4 |
| Used vCPU | 0.37 ~ 0.42 | 0.65 ~ 0.70 |

* This Library : [Performance report](https://storage.googleapis.com/0432808zbaeatxa/report_1637760853.008414.html) / [CPU Usage](https://storage.googleapis.com/0432808zbaeatxa/CPU%20(6).png)
* Google Cloud Go : [Performance report](https://storage.googleapis.com/0432808zbaeatxa/report_1637673736.2540932.html) / [CPU Usage](https://storage.googleapis.com/0432808zbaeatxa/CPU%20(5).png)

Test condition 
* 2.0 vCPU GKE Autopilot Pod
* 1 Node spanner database server
* 100 Users
* [Here](https://github.com/yoshidan/google-cloud-rust-example/commit/ccc484111bbd43d9642ee90ff27eca89e95ffe32) is the application for Load Test.