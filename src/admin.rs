use aws_sdk_dynamodb::model::AttributeValue;
use cookie::Cookie;
use lambda_http::{http::Method, RequestExt};
use serde::{Deserialize, Serialize};

use crate::{Handler, Output};

const COOKIE_NAME: &str = "admin_secret";
const KEY: &str = "key";
const URL: &str = "url";
const CURSOR: &str = "cursor";

impl<'a> Handler<'a> {
    pub async fn admin(mut self) -> Output {
        let request = match self.admin_request() {
            Ok(request) => request,
            Err(err) => return err,
        };

        let err = if let Some(AdminRequest::Login(req)) = &request {
            self.handle_login(&req)
        } else {
            self.authenticate()
        };

        match err {
            Some(err) => err,
            None => match request {
                Some(AdminRequest::Upsert(req)) => self.upsert(req).await,
                Some(AdminRequest::Delete(req)) => self.delete(req).await,
                _ => self.page_admin::<String>(None).await,
            },
        }
    }

    async fn upsert(self, req: UpsertRequest) -> Output {
        let err = self
            .client
            .put_item()
            .table_name(&self.config.table_name)
            .item(KEY, AttributeValue::S(req.key))
            .item(URL, AttributeValue::S(req.url))
            .send()
            .await
            .map(|_| None)
            .unwrap_or_else(|err| Some(err.to_string()));

        self.page_admin(err).await
    }

    async fn delete(self, req: DeleteRequest) -> Output {
        let err = self
            .client
            .delete_item()
            .table_name(&self.config.table_name)
            .key(KEY, AttributeValue::S(req.key))
            .send()
            .await
            .map(|_| None)
            .unwrap_or_else(|err| Some(err.to_string()));

        self.page_admin(err).await
    }

    async fn page_admin<E>(self, err: Option<E>) -> Output
    where
        E: AsRef<str>,
    {
        let query = self.event.query_string_parameters();
        let cursor = query.first(CURSOR).unwrap_or_default();

        let mut req = self.client.scan().table_name(&self.config.table_name);

        if !cursor.is_empty() {
            req = req.exclusive_start_key(KEY, AttributeValue::S(cursor.to_owned()));
        }

        let res = match req.send().await {
            Ok(res) => res,
            Err(err) => return self.render(500, self.render_error(err.to_string())),
        };
        let cursor = res.last_evaluated_key().and_then(|key| {
            key.get(KEY)
                .and_then(|key| match key {
                    AttributeValue::S(key) => Some(key),
                    _ => None,
                })
                .map(|key| key.to_owned())
        });

        let rows = res
            .items
            .unwrap_or_default()
            .into_iter()
            .flat_map(|mut item| {
                let key = item
                    .remove(KEY)
                    .and_then(|key| match key {
                        AttributeValue::S(key) => Some(key),
                        _ => None,
                    })
                    .unwrap_or_default();
                let url = item
                    .remove(URL)
                    .and_then(|url| match url {
                        AttributeValue::S(url) => Some(url),
                        _ => None,
                    })
                    .unwrap_or_default();
                if key.is_empty() || url.is_empty() {
                    None
                } else {
                    Some(format!(
                        include_str!("./templates/admin_row.html"),
                        key = key,
                        url = url
                    ))
                }
            })
            .collect::<Vec<_>>()
            .join("");

        self.render(
            200,
            format!(
                include_str!("./templates/admin.html"),
                error = err.map(|e| self.render_error(e)).unwrap_or_default(),
                rows = rows,
                nav = format!(
                    include_str!("./templates/nav.html"),
                    key = self.config.admin_key,
                    next = match cursor {
                        Some(cursor) => format!(
                            include_str!("./templates/nav_next.html"),
                            key = self.config.admin_key,
                            cursor = cursor
                        ),
                        None => "".to_owned(),
                    }
                ),
            ),
        )
    }

    fn admin_request(&self) -> Result<Option<AdminRequest>, Output> {
        if self.event.method() != Method::POST {
            Ok(None)
        } else {
            self.event
                .payload()
                .map_err(|err| self.render(400, self.render_error(err.to_string())))
        }
    }

    fn handle_login(&mut self, req: &LoginRequest) -> Option<Output> {
        if req.secret == self.config.admin_secret {
            self.add_cookie(
                Cookie::build(COOKIE_NAME, &req.secret)
                    .path("/")
                    .secure(true)
                    .http_only(true)
                    .finish(),
            );
            None
        } else {
            Some(self.page_login(true))
        }
    }

    fn authenticate(&self) -> Option<Output> {
        let secret = self
            .event
            .headers()
            .get("Cookie")
            .and_then(|cookies| cookies.to_str().ok())
            .and_then(|cookies| {
                Cookie::split_parse_encoded(cookies).find(|cookie| {
                    if let Ok(cookie) = cookie {
                        cookie.name() == COOKIE_NAME
                    } else {
                        false
                    }
                })
            })
            .and_then(|secret| secret.ok());
        let secret = secret.as_ref().map(|secret| secret.value());

        if secret == Some(&self.config.admin_secret) {
            None
        } else {
            // Render error page on failed login.
            Some(self.page_login(secret.is_some()))
        }
    }

    fn page_login(&self, invalid_secret: bool) -> Output {
        self.render(
            401,
            format!(
                include_str!("./templates/login.html"),
                error = if invalid_secret {
                    self.render_error("Secret is incorrect.")
                } else {
                    "".to_owned()
                }
            ),
        )
    }

    fn render_error(&self, err: impl AsRef<str>) -> String {
        format!(include_str!("./templates/error.html"), error = err.as_ref())
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum AdminRequest {
    Login(LoginRequest),
    Upsert(UpsertRequest),
    Delete(DeleteRequest),
}

#[derive(Debug, Serialize, Deserialize)]
struct LoginRequest {
    secret: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct UpsertRequest {
    key: String,
    url: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct DeleteRequest {
    key: String,
}