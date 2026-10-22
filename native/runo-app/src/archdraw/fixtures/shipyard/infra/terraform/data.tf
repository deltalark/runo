resource "aws_db_instance" "main" {
  identifier     = "shipyard-prod"
  engine         = "postgres"
  engine_version = "16.4"
  instance_class = "db.r6g.large"
}

resource "aws_elasticache_replication_group" "cache" {
  replication_group_id = "shipyard-cache"
  description          = "Sessions and rate limits"
  engine               = "redis"
  node_type            = "cache.t4g.medium"
}

resource "aws_opensearch_domain" "tracking" {
  domain_name    = "shipyard-tracking"
  engine_version = "OpenSearch_2.13"
}

resource "aws_s3_bucket" "assets" {
  bucket = "shipyard-assets"
}

resource "aws_s3_bucket" "exports" {
  bucket = "shipyard-exports"
}

resource "aws_sns_topic" "order_events" {
  name = "order-events"
}

resource "aws_sqs_queue" "shipments" {
  name = "shipments"
  redrive_policy = jsonencode({
    deadLetterTargetArn = aws_sqs_queue.shipments_dlq.arn
    maxReceiveCount     = 5
  })
}

resource "aws_sqs_queue" "shipments_dlq" {
  name = "shipments-dlq"
}

resource "aws_sqs_queue" "notifications" {
  name = "notifications"
}

resource "aws_sns_topic_subscription" "shipments" {
  topic_arn = aws_sns_topic.order_events.arn
  protocol  = "sqs"
  endpoint  = aws_sqs_queue.shipments.arn
}

resource "aws_sns_topic_subscription" "notifications" {
  topic_arn = aws_sns_topic.order_events.arn
  protocol  = "sqs"
  endpoint  = aws_sqs_queue.notifications.arn
}

resource "aws_lambda_function" "label_renderer" {
  function_name = "shipyard-label-renderer"
  runtime       = "nodejs20.x"
  handler       = "index.handler"
  environment {
    variables = {
      LABELS_BUCKET = aws_s3_bucket.exports.bucket
    }
  }
}

resource "aws_lambda_event_source_mapping" "labels" {
  event_source_arn = aws_sqs_queue.shipments.arn
  function_name    = aws_lambda_function.label_renderer.arn
}

resource "aws_secretsmanager_secret" "stripe" {
  name = "shipyard/stripe"
}

resource "aws_secretsmanager_secret" "sendgrid" {
  name = "shipyard/sendgrid"
}

resource "aws_secretsmanager_secret" "twilio" {
  name = "shipyard/twilio"
}
