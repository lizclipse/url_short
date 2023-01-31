mod admin;

use aws_sdk_dynamodb::{model::AttributeValue, Client};
use cookie::Cookie;
use lambda_http::{run, service_fn, Body, Error, Request, RequestExt, Response};

pub type Output = Result<Response<Body>, Error>;

pub struct Handler<'a> {
    client: &'a Client,
    config: &'a Config,
    event: Request,

    cookies: Vec<String>,
}

impl<'a> Handler<'a> {
    pub fn new<'b>(client: &'b Client, config: &'b Config, event: Request) -> Handler<'b> {
        Handler {
            client,
            config,
            event,

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

    async fn process_redirect(self, key: &str) -> Output {
        self.client
            .get_item()
            .table_name(&self.config.table_name)
            .key("key", AttributeValue::S(key.to_owned()))
            .projection_expression("url")
            .send()
            .await
            .map_err(Box::new)?
            .item()
            .ok_or((404, "No redirect was found"))
            .and_then(|item| {
                item.get("url")
                    .ok_or((500, "The URL for this redirect does not exist"))
            })
            .and_then(|url| {
                url.as_s()
                    .map_err(|_| (500, "The URL for this redirect is invalid"))
            })
            .map(|url| redirect_to(url))
            .unwrap_or_else(|(status, err)| self.render(status, err))
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
                    body = body.as_ref()
                )
                .into(),
            )
            .map_err(Box::new)?;
        Ok(resp)
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

    run(service_fn(|event| {
        Handler::new(&client, &config, event).run()
    }))
    .await?;

    // TODO: see if this is called consistently upon lambda shutdown
    tracing::info!("Shutting down");

    Ok(())
}
