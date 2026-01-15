// forked from https://github.com/diesel-rs/diesel_full_text_search/
mod types {
    use std::io::{BufRead, Cursor};

    use byteorder::{NetworkEndian, ReadBytesExt};
    use diesel::Queryable;
    use diesel::deserialize::FromSql;
    use diesel::pg::Pg;
    use diesel::sql_types::*;

    #[derive(Clone, Copy, SqlType)]
    #[diesel(postgres_type(oid = 3615, array_oid = 3645))]
    pub struct TsQuery;

    #[derive(Clone, Copy, SqlType)]
    #[diesel(postgres_type(oid = 3614, array_oid = 3643))]
    pub struct TsVector;
    pub type Tsvector = TsVector;

    pub trait TextOrNullableText {}

    impl TextOrNullableText for Text {}
    impl TextOrNullableText for Nullable<Text> {}
    impl TextOrNullableText for TsVector {}
    impl TextOrNullableText for Nullable<TsVector> {}

    #[derive(SqlType)]
    #[diesel(postgres_type(name = "regconfig"))]
    pub struct RegConfig;

    impl FromSql<TsVector, Pg> for PgTsVector {
        fn from_sql(
            bytes: <Pg as diesel::backend::Backend>::RawValue<'_>,
        ) -> diesel::deserialize::Result<Self> {
            let mut cursor = Cursor::new(bytes.as_bytes());

            // From Postgres `tsvector.c`:
            //
            //     The binary format is as follows:
            //
            //     uint32   number of lexemes
            //
            //     for each lexeme:
            //          lexeme text in client encoding, null-terminated
            //          uint16  number of positions
            //          for each position:
            //              uint16 WordEntryPos

            // Number of lexemes (uint32)
            let num_lexemes = cursor.read_u32::<NetworkEndian>()?;

            let mut entries = Vec::with_capacity(num_lexemes as usize);

            for _ in 0..num_lexemes {
                let mut lexeme = Vec::new();
                cursor.read_until(0, &mut lexeme)?;
                // Remove null terminator
                lexeme.pop();
                let lexeme = String::from_utf8(lexeme)?;

                // Number of positions (uint16)
                let num_positions = cursor.read_u16::<NetworkEndian>()?;

                let mut positions = Vec::with_capacity(num_positions as usize);
                for _ in 0..num_positions {
                    positions.push(cursor.read_u16::<NetworkEndian>()?);
                }

                entries.push(PgTsVectorEntry { lexeme, positions });
            }

            Ok(PgTsVector { entries })
        }
    }

    impl Queryable<TsVector, Pg> for PgTsVector {
        type Row = Self;

        fn build(row: Self::Row) -> diesel::deserialize::Result<Self> {
            Ok(row)
        }
    }

    #[derive(Debug, Clone, PartialEq)]
    pub struct PgTsVector {
        pub entries: Vec<PgTsVectorEntry>,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub struct PgTsVectorEntry {
        pub lexeme: String,
        pub positions: Vec<u16>,
    }
}

pub mod configuration {
    use diesel::backend::Backend;
    use diesel::deserialize::{self, FromSql, FromSqlRow};
    use diesel::expression::{ValidGrouping, is_aggregate};
    use diesel::pg::{Pg, PgValue};
    use diesel::query_builder::{AstPass, QueryFragment, QueryId};
    use diesel::serialize::{self, Output, ToSql};
    use diesel::sql_types::Integer;
    use diesel::{AppearsOnTable, Expression, QueryResult, SelectableExpression};

    use crate::full_text_search::RegConfig;

    #[derive(Debug, PartialEq, Eq, diesel::expression::AsExpression, FromSqlRow)]
    #[diesel(sql_type = RegConfig)]
    pub struct TsConfiguration(pub u32);

    impl TsConfiguration {
        pub const SIMPLE: Self = Self(3748);
        pub const DANISH: Self = Self(12824);
        pub const DUTCH: Self = Self(12826);
        pub const ENGLISH: Self = Self(12828);
        pub const FINNISH: Self = Self(12830);
        pub const FRENCH: Self = Self(12832);
        pub const GERMAN: Self = Self(12834);
        pub const HUNGARIAN: Self = Self(12836);
        pub const ITALIAN: Self = Self(12838);
        pub const NORWEGIAN: Self = Self(12840);
        pub const PORTUGUESE: Self = Self(12842);
        pub const ROMANIAN: Self = Self(12844);
        pub const RUSSIAN: Self = Self(12846);
        pub const SPANISH: Self = Self(12848);
        pub const SWEDISH: Self = Self(12850);
        pub const TURKISH: Self = Self(12852);
    }

    impl FromSql<RegConfig, Pg> for TsConfiguration
    where
        i32: FromSql<Integer, Pg>,
    {
        fn from_sql(bytes: PgValue) -> deserialize::Result<Self> {
            <i32 as FromSql<Integer, Pg>>::from_sql(bytes).map(|oid| TsConfiguration(oid as u32))
        }
    }

    impl ToSql<RegConfig, Pg> for TsConfiguration
    where
        i32: ToSql<Integer, Pg>,
    {
        fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Pg>) -> serialize::Result {
            <i32 as ToSql<Integer, Pg>>::to_sql(&(self.0 as i32), &mut out.reborrow())
        }
    }

    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub struct TsConfigurationByName(pub &'static str);

    impl<DB> QueryFragment<DB> for TsConfigurationByName
    where
        DB: Backend,
    {
        fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, DB>) -> QueryResult<()> {
            out.push_sql(&format!("'{}'", &self.0));
            Ok(())
        }
    }

    impl<GB> ValidGrouping<GB> for TsConfigurationByName {
        type IsAggregate = is_aggregate::Never;
    }

    impl QueryId for TsConfigurationByName {
        const HAS_STATIC_QUERY_ID: bool = false;

        type QueryId = ();
    }

    impl<QS> SelectableExpression<QS> for TsConfigurationByName where Self: Expression {}

    impl<QS> AppearsOnTable<QS> for TsConfigurationByName where Self: Expression {}

    impl Expression for TsConfigurationByName {
        type SqlType = RegConfig;
    }
}

#[allow(deprecated)]
mod functions {
    use diesel::define_sql_function;
    use diesel::sql_types::*;

    use crate::full_text_search::types::*;

    define_sql_function!(fn length(x: TsVector) -> Integer);
    define_sql_function!(fn numnode(x: TsQuery) -> Integer);
    define_sql_function!(fn plainto_tsquery(x: Text) -> TsQuery);
    define_sql_function! {
        #[sql_name = "plainto_tsquery"]
        fn plainto_tsquery_with_search_config(config: RegConfig, querytext: Text) -> TsQuery;
    }
    define_sql_function!(fn querytree(x: TsQuery) -> Text);
    define_sql_function!(fn strip(x: TsVector) -> TsVector);
    define_sql_function!(fn to_tsquery(x: Text) -> TsQuery);
    define_sql_function! {
        #[sql_name = "to_tsquery"]
        fn to_tsquery_with_search_config(config: RegConfig, querytext: Text) -> TsQuery;
    }
    define_sql_function!(fn to_tsvector<T: TextOrNullableText + SingleValue>(x: T) -> TsVector);
    define_sql_function! {
        #[sql_name = "to_tsvector"]
        fn to_tsvector_with_search_config<T: TextOrNullableText + SingleValue>(config: RegConfig, document_content: T) -> TsVector;
    }
    define_sql_function!(fn ts_headline(x: Text, y: TsQuery) -> Text);
    define_sql_function! {
        #[sql_name = "ts_headline"]
        fn ts_headline_with_search_config(config: RegConfig, x: Text, y: TsQuery) -> Text;
    }
    define_sql_function!(fn ts_rank(x: TsVector, y: TsQuery) -> Float);
    define_sql_function!(fn ts_rank_cd(x: TsVector, y: TsQuery) -> Float);
    define_sql_function! {
        #[sql_name = "ts_rank_cd"]
        fn ts_rank_cd_weighted(w: Array<Float>, x: TsVector, y: TsQuery) -> Float;
    }
    define_sql_function! {
        #[sql_name = "ts_rank_cd"]
        fn ts_rank_cd_normalized(x: TsVector, y: TsQuery, n: Integer) -> Float;
    }
    define_sql_function! {
        #[sql_name = "ts_rank_cd"]
        fn ts_rank_cd_weighted_normalized(w: Array<Float>, x: TsVector, y: TsQuery, n: Integer) -> Float;
    }
    define_sql_function!(fn phraseto_tsquery(x: Text) -> TsQuery);
    define_sql_function!(fn websearch_to_tsquery(x: Text) -> TsQuery);
    define_sql_function! {
        #[sql_name = "websearch_to_tsquery"]
        fn websearch_to_tsquery_with_search_config(config: RegConfig, x: Text) -> TsQuery;
    }
    define_sql_function!(fn setweight(x: TsVector, w: CChar) -> TsVector);
}

mod dsl {
    use diesel::expression::{AsExpression, Expression};

    use crate::full_text_search::types::*;

    mod predicates {
        use diesel::pg::Pg;

        use crate::full_text_search::types::*;

        diesel::infix_operator!(Matches, " @@ ", backend: Pg);
        diesel::infix_operator!(Concat, " || ", TsVector, backend: Pg);
        diesel::infix_operator!(And, " && ", TsQuery, backend: Pg);
        diesel::infix_operator!(Or, " || ", TsQuery, backend: Pg);
        diesel::infix_operator!(Contains, " @> ", backend: Pg);
        diesel::infix_operator!(ContainedBy, " <@ ", backend: Pg);
    }

    use self::predicates::*;

    pub trait TsVectorExtensions: Expression<SqlType = TsVector> + Sized {
        fn matches<T: AsExpression<TsQuery>>(self, other: T) -> Matches<Self, T::Expression> {
            Matches::new(self, other.as_expression())
        }

        fn concat<T: AsExpression<TsVector>>(self, other: T) -> Concat<Self, T::Expression> {
            Concat::new(self, other.as_expression())
        }
    }

    pub trait TsQueryExtensions: Expression<SqlType = TsQuery> + Sized {
        fn matches<T: AsExpression<TsVector>>(self, other: T) -> Matches<Self, T::Expression> {
            Matches::new(self, other.as_expression())
        }

        fn and<T: AsExpression<TsQuery>>(self, other: T) -> And<Self, T::Expression> {
            And::new(self, other.as_expression())
        }

        fn or<T: AsExpression<TsQuery>>(self, other: T) -> Or<Self, T::Expression> {
            Or::new(self, other.as_expression())
        }

        fn contains<T: AsExpression<TsQuery>>(self, other: T) -> Contains<Self, T::Expression> {
            Contains::new(self, other.as_expression())
        }

        fn contained_by<T: AsExpression<TsQuery>>(
            self,
            other: T,
        ) -> ContainedBy<Self, T::Expression> {
            ContainedBy::new(self, other.as_expression())
        }
    }

    impl<T: Expression<SqlType = TsVector>> TsVectorExtensions for T {}

    impl<T: Expression<SqlType = TsQuery>> TsQueryExtensions for T {}
}

pub use self::dsl::*;
pub use self::functions::*;
pub use self::types::*;

// mod tests {
//     use diesel::dsl::sql;

//     use crate::full_text_search::{PgTsVectorEntry, TsVector};

//     #[test]
//     fn test_tsvector_from_sql_with_positions() {
//         let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
//         let mut conn = diesel::PgConnection::establish(&database_url).expect("Error connecting to
// database");

//         let query = diesel::select(sql::<TsVector>(
//             "to_tsvector('a fat cat sat on a mat and ate a fat rat')",
//         ));
//         let result: PgTsVector = query.get_result(&mut conn).expect("Error executing query");

//         let expected = PgTsVector {
//             entries: vec![
//                 PgTsVectorEntry {
//                     lexeme: "ate".to_owned(),
//                     positions: vec![9],
//                 },
//                 PgTsVectorEntry {
//                     lexeme: "cat".to_owned(),
//                     positions: vec![3],
//                 },
//                 PgTsVectorEntry {
//                     lexeme: "fat".to_owned(),
//                     positions: vec![2, 11],
//                 },
//                 PgTsVectorEntry {
//                     lexeme: "mat".to_owned(),
//                     positions: vec![7],
//                 },
//                 PgTsVectorEntry {
//                     lexeme: "rat".to_owned(),
//                     positions: vec![12],
//                 },
//                 PgTsVectorEntry {
//                     lexeme: "sat".to_owned(),
//                     positions: vec![4],
//                 },
//             ],
//         };

//         assert_eq!(expected, result);
//     }

//     #[test]
//     fn test_tsvector_from_sql_without_positions() {
//         let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
//         let mut conn = PgConnection::establish(&database_url).expect("Error connecting to
// database");

//         let query = diesel::select(sql::<TsVector>("'a fat cat sat on a mat and ate a fat
// rat'::tsvector"));         let result: PgTsVector = query.get_result(&mut conn).expect("Error
// executing query");

//         let expected = PgTsVector {
//             entries: vec![
//                 PgTsVectorEntry {
//                     lexeme: "a".to_owned(),
//                     positions: vec![],
//                 },
//                 PgTsVectorEntry {
//                     lexeme: "and".to_owned(),
//                     positions: vec![],
//                 },
//                 PgTsVectorEntry {
//                     lexeme: "ate".to_owned(),
//                     positions: vec![],
//                 },
//                 PgTsVectorEntry {
//                     lexeme: "cat".to_owned(),
//                     positions: vec![],
//                 },
//                 PgTsVectorEntry {
//                     lexeme: "fat".to_owned(),
//                     positions: vec![],
//                 },
//                 PgTsVectorEntry {
//                     lexeme: "mat".to_owned(),
//                     positions: vec![],
//                 },
//                 PgTsVectorEntry {
//                     lexeme: "on".to_owned(),
//                     positions: vec![],
//                 },
//                 PgTsVectorEntry {
//                     lexeme: "rat".to_owned(),
//                     positions: vec![],
//                 },
//                 PgTsVectorEntry {
//                     lexeme: "sat".to_owned(),
//                     positions: vec![],
//                 },
//             ],
//         };

//         assert_eq!(expected, result);
//     }
// }
