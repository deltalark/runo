# Scheduled jobs: ECS tasks started by EventBridge rules, EventBridge Scheduler and a
# Step Functions pipeline.

resource "aws_ecr_repository" "jobs" {
  name = "shipyard-jobs"
}

resource "aws_ecs_task_definition" "nightly_report" {
  family                   = "${local.app}-nightly-report"
  requires_compatibilities = ["FARGATE"]
  container_definitions = jsonencode([{
    name  = "nightly-report"
    image = "${aws_ecr_repository.jobs.repository_url}:nightly-report"
    environment = [
      { name = "DATABASE_HOST", value = aws_db_instance.main.address },
      { name = "REPORTS_BUCKET", value = aws_s3_bucket.exports.bucket },
    ]
  }])
}

resource "aws_cloudwatch_event_rule" "nightly_report" {
  name                = "nightly-report"
  schedule_expression = "cron(0 2 * * ? *)"
}

resource "aws_cloudwatch_event_target" "nightly_report" {
  rule     = aws_cloudwatch_event_rule.nightly_report.name
  arn      = aws_ecs_cluster.main.arn
  role_arn = aws_iam_role.events.arn

  ecs_target {
    task_definition_arn = aws_ecs_task_definition.nightly_report.arn
    launch_type         = "FARGATE"
  }
}

resource "aws_ecs_task_definition" "invoice_run" {
  family                   = "${local.app}-invoice-run"
  requires_compatibilities = ["FARGATE"]
  container_definitions = jsonencode([{
    name  = "invoice-run"
    image = "${aws_ecr_repository.jobs.repository_url}:invoice-run"
    environment = [
      { name = "DATABASE_HOST", value = aws_db_instance.main.address },
      { name = "BILLING_URL", value = "http://billing.shipyard.local:7000" },
    ]
  }])
}

resource "aws_scheduler_schedule" "invoice_run" {
  name                = "invoice-run"
  schedule_expression = "cron(0 6 1 * ? *)"

  flexible_time_window {
    mode = "OFF"
  }

  target {
    arn      = aws_ecs_cluster.main.arn
    role_arn = aws_iam_role.scheduler.arn

    ecs_parameters {
      task_definition_arn = aws_ecs_task_definition.invoice_run.arn
      launch_type         = "FARGATE"
    }
  }
}

resource "aws_ecs_task_definition" "carrier_sync" {
  family                   = "${local.app}-carrier-sync"
  requires_compatibilities = ["FARGATE"]
  container_definitions = jsonencode([{
    name  = "carrier-sync"
    image = "${aws_ecr_repository.jobs.repository_url}:carrier-sync"
    environment = [
      { name = "QUEUE_URL", value = aws_sqs_queue.shipments.url },
      { name = "TRACKING_URL", value = "http://tracking.shipyard.local:9100" },
    ]
  }])
}

resource "aws_scheduler_schedule" "carrier_sync" {
  name                = "carrier-sync"
  schedule_expression = "rate(15 minutes)"

  flexible_time_window {
    mode = "OFF"
  }

  target {
    arn      = aws_ecs_cluster.main.arn
    role_arn = aws_iam_role.scheduler.arn

    ecs_parameters {
      task_definition_arn = aws_ecs_task_definition.carrier_sync.arn
    }
  }
}

resource "aws_ecs_task_definition" "data_export" {
  family                   = "${local.app}-data-export"
  requires_compatibilities = ["FARGATE"]
  container_definitions = jsonencode([{
    name  = "data-export"
    image = "${aws_ecr_repository.jobs.repository_url}:data-export"
    environment = [
      { name = "DATABASE_HOST", value = aws_db_instance.main.address },
      { name = "EXPORT_BUCKET", value = aws_s3_bucket.exports.bucket },
    ]
  }])
}

resource "aws_sfn_state_machine" "data_pipeline" {
  name     = "data-pipeline"
  role_arn = aws_iam_role.sfn.arn
  definition = jsonencode({
    StartAt = "Export"
    States = {
      Export = {
        Type     = "Task"
        Resource = "arn:aws:states:::ecs:runTask.sync"
        Parameters = {
          Cluster        = aws_ecs_cluster.main.arn
          TaskDefinition = aws_ecs_task_definition.data_export.arn
        }
        End = true
      }
    }
  })
}

resource "aws_cloudwatch_event_rule" "weekly_export" {
  name                = "weekly-export"
  schedule_expression = "cron(0 3 ? * SUN *)"
}

resource "aws_cloudwatch_event_target" "weekly_export" {
  rule     = aws_cloudwatch_event_rule.weekly_export.name
  arn      = aws_sfn_state_machine.data_pipeline.arn
  role_arn = aws_iam_role.events.arn
}

resource "aws_iam_role" "events" {
  name = "shipyard-events"
}

resource "aws_iam_role" "scheduler" {
  name = "shipyard-scheduler"
}

resource "aws_iam_role" "sfn" {
  name = "shipyard-sfn"
}
