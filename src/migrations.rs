//! Migrations embarquées dans le binaire : aucune base ni dossier `migrations/` au run.
//!
//! Même moteur que `sqlx::migrate!` (table `_sqlx_migrations`, sommes de contrôle, verrou
//! consultatif entre instances), sans la proc-macro, qui recompile sqlx, rustls et ring
//! une seconde fois au build. Une nouvelle migration = une ligne dans `MIGRATIONS`.

use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;

use sqlx::error::BoxDynError;
use sqlx::migrate::{Migration, MigrationSource, MigrationType, Migrator};
use sqlx::PgPool;

const MIGRATIONS: [(i64, &str, &str); 1] = [(1, "schema", include_str!("../migrations/0001_schema.sql"))];

#[derive(Debug)]
struct Embarquees;

impl MigrationSource<'static> for Embarquees {
    fn resolve(self) -> Pin<Box<dyn Future<Output = Result<Vec<Migration>, BoxDynError>> + Send>> {
        Box::pin(async {
            Ok(MIGRATIONS
                .iter()
                .map(|(version, nom, sql)| {
                    Migration::new(
                        *version,
                        Cow::Borrowed(*nom),
                        MigrationType::Simple,
                        Cow::Borrowed(*sql),
                        false,
                    )
                })
                .collect())
        })
    }
}

pub async fn appliquer(pool: &PgPool) -> Result<(), String> {
    let migrator = Migrator::new(Embarquees).await.map_err(|e| e.to_string())?;
    migrator.run(pool).await.map_err(|e| e.to_string())
}
