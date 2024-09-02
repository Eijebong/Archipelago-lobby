use ap_lobby::db::Json;
use ap_lobby::error::{Error, Result};
use ap_lobby::extractor::extract_features;
use ap_lobby::{db::YamlFile, schema::yamls};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::AsyncConnection;
use diesel_async::{
    pooled_connection::{deadpool::Pool, AsyncDieselConnectionManager},
    AsyncPgConnection, RunQueryDsl,
};
use dotenvy::dotenv;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "debug");
    }

    let db_url = std::env::var("DATABASE_URL").expect("Plox provide a DATABASE_URL env variable");
    let mgr = AsyncDieselConnectionManager::<AsyncPgConnection>::new(db_url);
    let db_pool = Pool::builder(mgr)
        .build()
        .expect("Failed to create database pool, aborting");

    let mut conn = db_pool.get().await?;

    let all_yamls: Vec<(Uuid, String)> = yamls::table
        .select((yamls::id, yamls::content))
        .load(&mut conn)
        .await?;
    conn.transaction::<(), Error, _>(|mut conn| {
        async move {
            for (yaml_id, raw_yaml) in &all_yamls {
                let Ok(parsed) =
                    serde_yaml::from_str::<YamlFile>(raw_yaml.trim_start_matches('\u{feff}'))
                else {
                    continue;
                };
                let Ok(features) = extract_features(&parsed, raw_yaml) else {
                    continue;
                };

                diesel::update(yamls::table.find(yaml_id))
                    .set(yamls::features.eq(Json(features)))
                    .execute(&mut conn)
                    .await?;
            }

            Ok(())
        }
        .scope_boxed()
    })
    .await?;

    Ok(())
}
