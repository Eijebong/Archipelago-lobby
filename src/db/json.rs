use diesel::deserialize::FromSqlRow;
use diesel::expression::AsExpression;
use diesel::pg::{Pg, PgValue};
use diesel::sql_types;
use diesel::{deserialize::FromSql, serialize::ToSql};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde::Serialize;
use std::ops::Deref;

#[derive(FromSqlRow, AsExpression, Serialize, Deserialize, Debug, Clone)]
#[serde(transparent)]
#[diesel(sql_type = sql_types::Json)]
#[diesel(sql_type = sql_types::Jsonb)]
pub struct Json<T: Sized>(pub T);

impl<T> Deref for Json<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> FromSql<sql_types::Json, Pg> for Json<T>
where
    T: std::fmt::Debug + DeserializeOwned,
{
    fn from_sql(bytes: PgValue) -> diesel::deserialize::Result<Self> {
        let value = <serde_json::Value as FromSql<sql_types::Json, Pg>>::from_sql(bytes)?;
        Ok(Json(serde_json::from_value::<T>(value)?))
    }
}

impl<T> ToSql<sql_types::Json, Pg> for Json<T>
where
    T: std::fmt::Debug + Serialize,
{
    fn to_sql(&self, out: &mut diesel::serialize::Output<Pg>) -> diesel::serialize::Result {
        let value = serde_json::to_value(self)?;
        <serde_json::Value as ToSql<sql_types::Json, Pg>>::to_sql(&value, &mut out.reborrow())
    }
}

impl<T> FromSql<sql_types::Jsonb, Pg> for Json<T>
where
    T: std::fmt::Debug + DeserializeOwned,
{
    fn from_sql(bytes: PgValue) -> diesel::deserialize::Result<Self> {
        let value = <serde_json::Value as FromSql<sql_types::Jsonb, Pg>>::from_sql(bytes)?;
        Ok(Json(serde_json::from_value::<T>(value)?))
    }
}

impl<T> ToSql<sql_types::Jsonb, Pg> for Json<T>
where
    T: std::fmt::Debug + Serialize,
{
    fn to_sql(&self, out: &mut diesel::serialize::Output<Pg>) -> diesel::serialize::Result {
        let value = serde_json::to_value(self)?;
        <serde_json::Value as ToSql<sql_types::Jsonb, Pg>>::to_sql(&value, &mut out.reborrow())
    }
}
