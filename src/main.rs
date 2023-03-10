mod admin;
mod hit;

use aws_sdk_dynamodb::{model::AttributeValue, Client};
use cookie::Cookie;
use futures::join;
use hit::HitTrackerSender;
use lambda_http::{run, service_fn, Body, Error, Request, RequestExt, Response};

use crate::hit::hit_tracker;

pub type Output = Result<Response<Body>, Error>;

const KEY: &str = "key";
const URL: &str = "redirect_url";
const CREATED: &str = "created";
const UPDATED: &str = "updated";
const HITS: &str = "hits";

pub struct Handler<'a> {
    client: &'a Client,
    config: &'a Config,
    event: Request,
    tracker: HitTrackerSender,

    cookies: Vec<String>,
}

impl<'a> Handler<'a> {
    pub fn new<'b>(
        client: &'b Client,
        config: &'b Config,
        event: Request,
        tracker: &'b HitTrackerSender,
    ) -> Handler<'b> {
        Handler {
            client,
            config,
            event,
            tracker: tracker.clone(),

            cookies: vec![],
        }
    }

    async fn run(self) -> Output {
        let params = self.event.path_parameters();
        let key_param = params.first(&self.config.key_param);

        match key_param {
            Some("") | None => redirect_to(&self.config.default_redirect),
            Some(key) => {
                if key == self.config.admin_key {
                    self.admin().await
                } else {
                    self.process_redirect(key).await
                }
            }
        }
    }

    async fn process_redirect(mut self, key: &str) -> Output {
        match self
            .client
            .get_item()
            .table_name(&self.config.table_name)
            .key(KEY, AttributeValue::S(key.to_owned()))
            .projection_expression(URL)
            .send()
            .await
            .map_err(|err| {
                tracing::warn!("Failed to get item: {:?}", err);
                (500, err.to_string())
            })
            .and_then(|res| res.item.ok_or((404, "No redirect was found".to_owned())))
            .and_then(|mut item| {
                item.remove(URL)
                    .ok_or((500, "The URL for this redirect does not exist".to_owned()))
            })
            .and_then(|url| match url {
                AttributeValue::S(url) => Ok(url),
                _ => Err((500, "The URL for this redirect is invalid".to_owned())),
            }) {
            Ok(url) => {
                self.tracker.track(key.to_owned()).await;
                redirect_to(url)
            }
            Err((status, err)) => self.render(status, self.render_error(err)),
        }
    }

    fn render(&self, status: u16, body: impl AsRef<str>) -> Output {
        let mut resp = Response::builder()
            .status(status)
            .header("Content-Type", "text/html");

        for cookie in self.cookies.iter() {
            resp = resp.header("Set-Cookie", cookie);
        }

        let resp = resp
            .body(
                format!(
                    include_str!("./templates/_layout.html"),
                    title = "Url Shortener",
                    css = format!("<style>{}</style>", include_str!("./templates/_layout.css")),
                    body = body.as_ref()
                )
                .into(),
            )
            .map_err(Box::new)?;
        Ok(resp)
    }

    fn render_error(&self, err: impl AsRef<str>) -> String {
        format!(include_str!("./templates/error.html"), error = err.as_ref())
    }

    fn add_cookie(&mut self, cookie: Cookie) {
        self.cookies.push(cookie.encoded().to_string());
    }
}

fn redirect_to(url: impl AsRef<str>) -> Output {
    let resp = Response::builder()
        .status(301)
        .header("Location", url.as_ref())
        .body(().into())
        .map_err(Box::new)?;
    Ok(resp)
}

pub struct Config {
    table_name: String,
    key_param: String,
    default_redirect: String,
    admin_key: String,
    admin_secret: String,
}

impl Config {
    fn new() -> Result<Self, Error> {
        let table_name = std::env::var("TABLE_NAME")?;
        let key_param = std::env::var("KEY_PARAM")?;
        let default_redirect = std::env::var("DEFAULT_REDIRECT")?;
        let admin_key = std::env::var("ADMIN_KEY")?;
        let admin_secret = std::env::var("ADMIN_SECRET")?;
        Ok(Self {
            table_name,
            key_param,
            default_redirect,
            admin_key,
            admin_secret,
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        // disable printing the name of the module in every log line.
        .with_target(false)
        // disabling time is handy because CloudWatch will add the ingestion time.
        .without_time()
        .init();

    let sdk_config = aws_config::load_from_env().await;
    let client = Client::new(&sdk_config);
    let config = Config::new()?;
    let (hit_tx, mut hit_rx) = hit_tracker(&client, &config);

    let (res, _) = join!(
        async {
            let res = run(service_fn(|event| {
                Handler::new(&client, &config, event, &hit_tx).run()
            }))
            .await;
            // Close the sender so that the receiver will stop and let the join complete.
            hit_tx.close();
            res
        },
        hit_rx.run()
    );
    res?;

    Ok(())
}
