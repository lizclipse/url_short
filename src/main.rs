use aws_sdk_dynamodb::{model::AttributeValue, Client};
use lambda_http::{run, service_fn, Body, Error, Request, RequestExt, Response};

async fn function_handler(
    client: &Client,
    config: &Config,
    event: Request,
) -> Result<Response<Body>, Error> {
    let params = event.path_parameters();
    let key_param = params.first(&config.key_param);

    match key_param {
        Some("") | None => redirect_to(&config.default_redirect),
        Some(key) => client
            .get_item()
            .table_name(&config.table_name)
            .key("key", AttributeValue::S(key.to_owned()))
            .projection_expression("url")
            .send()
            .await
            .map_err(Box::new)?
            .item()
            .ok_or("No redirect was found")
            .and_then(|item| {
                item.get("url")
                    .ok_or("The URL for this redirect does not exist")
            })
            .and_then(|url| {
                url.as_s()
                    .map_err(|_| "The URL for this redirect is invalid")
            })
            .map(|url| redirect_to(url))
            .unwrap_or_else(|err| render_page(404, err)),
    }
}

fn redirect_to(url: impl AsRef<str>) -> Result<Response<Body>, Error> {
    let resp = Response::builder()
        .status(301)
        .header("Location", url.as_ref())
        .body(().into())
        .map_err(Box::new)?;
    Ok(resp)
}

fn render_page(status: u16, message: impl AsRef<str>) -> Result<Response<Body>, Error> {
    // TODO: Move template to a file
    let resp = Response::builder()
        .status(status)
        .header("Content-Type", "text/html")
        .body(
            format!(
                "
<!DOCTYPE html>
<html>
<head>
    <meta charset=\"utf-8\">
    <title>Url Shortener</title>
</head>
<body>
    <p>{}</p>
</body>
</html>
",
                message.as_ref()
            )
            .into(),
        )
        .map_err(Box::new)?;
    Ok(resp)
}

struct Config {
    table_name: String,
    key_param: String,
    default_redirect: String,
}

impl Config {
    fn new() -> Result<Self, Error> {
        let table_name = std::env::var("TABLE_NAME")?;
        let key_param = std::env::var("KEY_PARAM")?;
        let default_redirect = std::env::var("DEFAULT_REDIRECT")?;
        Ok(Self {
            table_name,
            key_param,
            default_redirect,
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
        function_handler(&client, &config, event)
    }))
    .await
}
