use std::path::PathBuf;

use ap_lobby::db::{Json, YamlId};
use ap_lobby::error::{Error, Result};
use ap_lobby::extractor::extract_features;
use ap_lobby::{db::YamlFile, schema::yamls};
use apwm::Index;
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::AsyncConnection;
use diesel_async::{
    pooled_connection::{deadpool::Pool, AsyncDieselConnectionManager},
    AsyncPgConnection, RunQueryDsl,
};
use dotenvy::dotenv;

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "debug");
    }

    let db_url = std::env::var("DATABASE_URL").expect("Plox provide a DATABASE_URL env variable");
    let index_path = std::env::var("INDEX_PATH").expect("Plox provide an INDEX_PATH env variable");
    let mgr = AsyncDieselConnectionManager::<AsyncPgConnection>::new(db_url);
    let db_pool = Pool::builder(mgr)
        .build()
        .expect("Failed to create database pool, aborting");

    let index = Index::new(&PathBuf::from(index_path))?;
    let mut conn = db_pool.get().await?;

    let all_yamls: Vec<(YamlId, String)> = yamls::table
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

                let Ok(features) = extract_features(&index, &parsed, raw_yaml) else {
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
