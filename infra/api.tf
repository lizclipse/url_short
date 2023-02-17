locals {
  name     = "${var.name}_${random_uuid.name.result}"
  dist_zip = "${path.module}/../target/lambda/url_short/bootstrap.zip"

  tags = {
    Project = "url_short"
    Name    = local.name
  }
}

resource "random_uuid" "name" {}

module "dynamodb_table" {
  source = "terraform-aws-modules/dynamodb-table/aws"

  name                = local.name
  hash_key            = "key"
  billing_mode        = "PROVISIONED"
  read_capacity       = 1
  write_capacity      = 1
  autoscaling_enabled = true

  autoscaling_read = {
    max_capacity = 10
  }

  autoscaling_write = {
    max_capacity = 10
  }

  attributes = [
    {
      name = "key"
      type = "S"
    }
  ]

  tags = local.tags
}

module "lambda_function" {
  source = "terraform-aws-modules/lambda/aws"

  function_name = local.name
  description   = "Core url_short function"
  runtime       = "provided.al2"
  handler       = "main"
  memory_size   = 256
  architectures = ["arm64"]

  publish                = true
  create_package         = false
  local_existing_package = local.dist_zip

  allowed_triggers = {
    AllowExecutionFromAPIGateway = {
      service    = "apigateway"
      source_arn = "${module.api_gateway.apigatewayv2_api_execution_arn}/*/*"
    }
  }

  environment_variables = {
    TABLE_NAME       = local.name
    KEY_PARAM        = "key"
    DEFAULT_REDIRECT = var.default_redirect
    ADMIN_KEY        = var.admin_key
    ADMIN_SECRET     = var.admin_secret
  }

  attach_policy_statements = true
  policy_statements = [
    {
      sid    = "AllowDynamoDB"
      effect = "Allow"

      actions = [
        "dynamodb:*",
      ]

      resources = [
        module.dynamodb_table.dynamodb_table_arn,
      ]
    }
  ]

  tags = local.tags
}

module "api_gateway" {
  source = "terraform-aws-modules/apigateway-v2/aws"

  name          = local.name
  description   = "url_short API"
  protocol_type = "HTTP"

  cors_configuration = {
    allow_headers = ["content-type", "x-amz-date", "authorization", "x-api-key", "x-amz-security-token", "x-amz-user-agent"]
    allow_methods = ["*"]
    allow_origins = ["*"]
  }

  # Custom domain
  domain_name                 = var.domain
  domain_name_certificate_arn = module.acm.acm_certificate_arn

  default_stage_access_log_destination_arn = aws_cloudwatch_log_group.logs.arn
  default_stage_access_log_format          = "$context.identity.sourceIp - - [$context.requestTime] \"$context.httpMethod $context.routeKey $context.protocol\" $context.status $context.responseLength $context.requestId $context.integrationErrorMessage"

  # Routes and integrations
  integrations = {
    "ANY /{key}" = {
      lambda_arn             = module.lambda_function.lambda_function_arn
      payload_format_version = "2.0"
    }

    "$default" = {
      lambda_arn             = module.lambda_function.lambda_function_arn
      payload_format_version = "2.0"
    }
  }

  tags = local.tags
}

resource "aws_cloudwatch_log_group" "logs" {
  name = local.name
}

data "aws_route53_zone" "host" {
  name         = var.base_domain == null ? var.domain : var.base_domain
  private_zone = false
}

resource "aws_route53_record" "api" {
  zone_id = data.aws_route53_zone.host.zone_id
  name    = var.domain
  type    = "A"

  alias {
    name                   = module.api_gateway.apigatewayv2_domain_name_configuration[0].target_domain_name
    zone_id                = module.api_gateway.apigatewayv2_domain_name_configuration[0].hosted_zone_id
    evaluate_target_health = false
  }
}

module "acm" {
  source  = "terraform-aws-modules/acm/aws"
  version = "~> 4.0"

  domain_name = var.base_domain == null ? var.domain : var.base_domain
  zone_id     = data.aws_route53_zone.host.id

  subject_alternative_names = var.base_domain == null ? [] : [var.domain]

  wait_for_validation = true

  tags = local.tags
}
