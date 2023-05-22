use std::net::SocketAddr;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{ErrorResponse, IntoResponse, Redirect},
    routing::{get, post},
    Json, Router, TypedHeader,
};
use axum_client_ip::{InsecureClientIp, SecureClientIpSource};
use diesel::prelude::*;
use headers::ContentType;
use models::NewUrl;
use nanoid::nanoid;
use schema::urls;
use serde::{Deserialize, Serialize};
use tower::ServiceBuilder;
use tower_http::trace::TraceLayer;
use tracing::warn;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::models::Url;

pub mod models;
pub mod schema;

pub fn gen_slug() -> String {
    nanoid!(10)
}

#[derive(Debug)]
pub enum UrlErr {
    SlugOccupied,
    SlugTooManyTries,
    DBError,
    JsonError(serde_json::Error),
    NotFound,
}

impl IntoResponse for UrlErr {
    fn into_response(self) -> axum::response::Response {
        let (res, status) = match self {
            UrlErr::SlugOccupied => (
                "This slug is already in use.".to_string(),
                StatusCode::CONFLICT,
            ),
            UrlErr::SlugTooManyTries => (
                "Unable to find a random slug to use, try again later.".to_string(),
                StatusCode::REQUEST_TIMEOUT,
            ),
            UrlErr::DBError => (
                "There was an error with the database.".to_string(),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            UrlErr::JsonError(err) => (
                format!("Error parsing json: {}", err),
                StatusCode::BAD_REQUEST,
            ),
            UrlErr::NotFound => (
                "Shortened URL not found.".to_string(),
                StatusCode::NOT_FOUND,
            ),
        };

        #[derive(Debug, Serialize)]
        struct Error {
            message: String,
        }

        let mut res = Json(Error { message: res }).into_response();
        let s = res.status_mut();
        *s = status;
        res
    }
}

async fn create_url(
    url: String,
    slug: Option<String>,
    author_ip: String,
    pool: deadpool_diesel::sqlite::Pool,
) -> Result<Url, UrlErr> {
    let conn = pool.get().await.unwrap();
    conn.interact(move |conn| {
        let mut collides = |try_slug| {
            use self::schema::urls::dsl::*;
            let result = urls.filter(slug.eq(try_slug)).limit(1).load::<Url>(conn);
            if let Ok(v) = result {
                v.len() > 0
            } else {
                true // There's been some other error, so let's just pretend that it's colliding
            }
        };

        let new_slug = if let Some(slug) = slug {
            if collides(slug.clone()) {
                return Err(UrlErr::SlugOccupied);
            }
            slug
        } else {
            let mut slug = Some(gen_slug());
            for _ in 0..10 {
                slug = Some(gen_slug());
                if !collides(slug.clone().unwrap()) {
                    break;
                }
                slug = None;
            }

            match slug {
                Some(slug) => slug,
                None => return Err(UrlErr::SlugTooManyTries),
            }
        };

        let np = NewUrl {
            slug: &new_slug,
            url: &url,
            author_ip: &author_ip,
            usage_count: 0,
        };
        diesel::insert_into(urls::table)
            .values(np)
            //.returning(Url::as_returning())
            .execute(conn)
            .map_err(|_| UrlErr::DBError)?;

        let new_url = {
            use self::schema::urls::dsl::*;
            urls.filter(slug.eq(new_slug))
                .limit(1)
                .load::<Url>(conn)
                .map_err(|_| UrlErr::DBError)?
        };
        Ok(new_url.get(0).cloned().unwrap())
    })
    .await
    .map_err(|_| UrlErr::DBError)?
}

async fn post_root(
    State(pool): State<deadpool_diesel::sqlite::Pool>,
    content_type: Option<TypedHeader<ContentType>>,
    InsecureClientIp(ip): InsecureClientIp,
    body: String,
) -> Result<Json<Url>, ErrorResponse> {
    let (url, slug) = if let Some(TypedHeader(ct)) = content_type {
        if ct == ContentType::json() {
            let json = serde_json::from_str::<ShortReq>(&body).map_err(UrlErr::JsonError)?;
            (json.url, json.slug)
        } else {
            (body.clone(), None)
        }
    } else {
        (body.clone(), None)
    };

    let author_ip = format!("{:?}", ip);

    let entry = create_url(url, slug, author_ip, pool);
    Ok(Json(entry.await?))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ShortReq {
    url: String,
    slug: Option<String>,
}

async fn get_redir(
    State(pool): State<deadpool_diesel::sqlite::Pool>,
    Path(slug_id): Path<String>,
) -> Result<Redirect, UrlErr> {
    let conn = pool.get().await.unwrap();
    let url: Result<String, UrlErr> = conn
        .interact(move |conn| {
            use self::schema::urls::dsl::*;

            let result = urls
                .filter(slug.eq(&slug_id))
                .limit(1)
                .load::<Url>(conn)
                .map_err(|_| UrlErr::DBError)?;

            if result.len() == 0 {
                return Err(UrlErr::NotFound);
            } else {
                diesel::update(urls.find(&slug_id))
                    .set(usage_count.eq(usage_count + 1))
                    .execute(conn)
                    .map_err(|_| warn!("Unable to update `usage_count` for {}", slug_id))
                    .unwrap();
                return Ok(result[0].url.clone());
            }
        })
        .await
        .unwrap();
    url.map(|ref s| Redirect::to(s))
}

#[tokio::main]
async fn main() {
    let db_url = "sqlite://db/db.sqlite";

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "example_todos=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // set up connection pool
    let manager = deadpool_diesel::sqlite::Manager::new(db_url, deadpool_diesel::Runtime::Tokio1);
    let pool = deadpool_diesel::sqlite::Pool::builder(manager)
        .build()
        .unwrap();

    // build our application with a single route
    let app = Router::new()
        .route("/", post(post_root))
        .route("/:slug", get(get_redir))
        .layer(
            ServiceBuilder::new()
                .layer(TraceLayer::new_for_http())
                .into_inner(),
        )
        .layer(SecureClientIpSource::ConnectInfo.into_extension())
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(pool);

    // run it with hyper on localhost:3000
    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .unwrap();
}
