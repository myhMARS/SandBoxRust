use actix_web::{web, FromRequest, HttpRequest};
use std::future::{ready, Ready};

/// Extracts and validates the `X-Api-Key` header.
///
/// Usage: add `ApiKey` as a handler parameter and it will be extracted
/// by actix-web's type-safe extractor system (equivalent to FastAPI's `Depends`).
pub struct ApiKey(#[allow(dead_code)] pub String);

impl FromRequest for ApiKey {
    type Error = actix_web::Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _: &mut actix_web::dev::Payload) -> Self::Future {
        let config = req
            .app_data::<web::Data<crate::config::Config>>()
            .expect("Config not in app_data");

        match req.headers().get("X-Api-Key").and_then(|v| v.to_str().ok()) {
            Some(val) if val == config.app.key => ready(Ok(ApiKey(val.into()))),
            _ => ready(Err(actix_web::error::ErrorUnauthorized(
                r#"{"code":401,"message":"Invalid API key","data":null}"#,
            ))),
        }
    }
}
