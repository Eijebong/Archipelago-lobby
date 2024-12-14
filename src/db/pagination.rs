use diesel::pg::Pg;
use diesel::prelude::*;
use diesel::query_builder::*;
use diesel::sql_types::BigInt;

use diesel_async::methods::LoadQuery;
use diesel_async::AsyncPgConnection;
use diesel_async::RunQueryDsl;

pub trait Paginate: Sized {
    fn paginate(self, page: u64) -> Paginated<Self>;
}

impl<T> Paginate for T {
    fn paginate(self, page: u64) -> Paginated<Self> {
        let page = if page == 0 { 1 } else { page as i64 };

        Paginated {
            query: self,
            per_page: DEFAULT_PER_PAGE,
            offset: (page - 1) * DEFAULT_PER_PAGE,
        }
    }
}

const DEFAULT_PER_PAGE: i64 = 10;

#[derive(Debug, Clone, Copy, QueryId)]
pub struct Paginated<T> {
    query: T,
    per_page: i64,
    offset: i64,
}

impl<T: Query> Paginated<T> {
    pub fn load_and_count_pages<'a, U>(
        self,
        conn: &'a mut AsyncPgConnection,
    ) -> impl std::future::Future<Output = QueryResult<(Vec<U>, u64)>> + Send + 'a
    where
        Self: LoadQuery<'a, AsyncPgConnection, (U, i64)>,
        U: Send + 'a,
        T: 'a,
    {
        let per_page = self.per_page;
        let results = self.load::<(U, i64)>(conn);

        async move {
            let results = results.await?;
            #[allow(clippy::get_first)]
            let total = results.get(0).map(|x| x.1).unwrap_or(0);
            let records = results.into_iter().map(|x| x.0).collect();
            let total_pages = (total as f64 / per_page as f64).ceil() as u64;
            Ok((records, total_pages))
        }
    }
}

impl<T: Query> Query for Paginated<T> {
    type SqlType = (T::SqlType, BigInt);
}

impl<T> QueryFragment<Pg> for Paginated<T>
where
    T: QueryFragment<Pg>,
{
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Pg>) -> QueryResult<()> {
        out.push_sql("SELECT *, COUNT(*) OVER () FROM (");
        self.query.walk_ast(out.reborrow())?;
        out.push_sql(") t LIMIT ");
        out.push_bind_param::<BigInt, _>(&self.per_page)?;
        out.push_sql(" OFFSET ");
        out.push_bind_param::<BigInt, _>(&self.offset)?;
        Ok(())
    }
}
