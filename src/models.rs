use crate::schema::urls;
use diesel::prelude::*;
use serde::Serialize;

#[derive(Selectable, Queryable, Serialize, Debug, Clone)]
pub struct Url {
    pub slug: String,
    pub url: String,
    pub author_ip: String,
    pub usage_count: i32,
}

#[derive(Insertable, Clone)]
#[diesel(table_name = urls)]
pub struct NewUrl<'a> {
    pub slug: &'a str,
    pub url: &'a str,
    pub author_ip: &'a str,
    pub usage_count: i32,
}
