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
export SPANNER_EMULATOR_HOST=localhost:9012
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

See [`examples/omni.rs`](examples/omni.rs) for a runnable end-to-end example, including the
`docker run` lines that bring up a container to run it against.

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