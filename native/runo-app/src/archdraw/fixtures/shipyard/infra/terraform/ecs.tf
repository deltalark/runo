resource "aws_ecs_cluster" "main" {
  name = "${local.app}-prod"
}

resource "aws_service_discovery_private_dns_namespace" "internal" {
  name = "shipyard.local"
  vpc  = aws_vpc.main.id
}

resource "aws_ecs_task_definition" "web" {
  family                   = "${local.app}-web"
  requires_compatibilities = ["FARGATE"]
  cpu                      = 512
  memory                   = 1024
  container_definitions = jsonencode([
    {
      name  = "web"
      image = "${aws_ecr_repository.web.repository_url}:latest"
      environment = [
        { name = "API_URL", value = "https://shipyard.example/api" },
      ]
    }
  ])
}

resource "aws_ecr_repository" "web" {
  name = "web"
}

resource "aws_ecs_service" "web" {
  name            = "web"
  cluster         = aws_ecs_cluster.main.id
  task_definition = aws_ecs_task_definition.web.arn
  desired_count   = 2
  launch_type     = "FARGATE"

  network_configuration {
    subnets = aws_subnet.private[*].id
  }

  load_balancer {
    target_group_arn = aws_lb_target_group.web.arn
    container_name   = "web"
  }
}

resource "aws_ecs_task_definition" "api" {
  family                   = "${local.app}-api"
  requires_compatibilities = ["FARGATE"]
  cpu                      = 512
  memory                   = 1024
  container_definitions = jsonencode([
    {
      name  = "api"
      image = "${aws_ecr_repository.api.repository_url}:latest"
      environment = [
        { name = "DATABASE_HOST", value = aws_db_instance.main.address },
        { name = "REDIS_HOST", value = aws_elasticache_replication_group.cache.primary_endpoint_address },
        { name = "ORDERS_URL", value = "http://orders.shipyard.local:9000" },
        { name = "TRACKING_URL", value = "http://tracking.shipyard.local:9100" },
        { name = "ASSETS_BUCKET", value = aws_s3_bucket.assets.bucket },
      ]
    }
  ])
}

resource "aws_ecr_repository" "api" {
  name = "api"
}

resource "aws_ecs_service" "api" {
  name            = "api"
  cluster         = aws_ecs_cluster.main.id
  task_definition = aws_ecs_task_definition.api.arn
  desired_count   = 2
  launch_type     = "FARGATE"

  network_configuration {
    subnets = aws_subnet.private[*].id
  }

  load_balancer {
    target_group_arn = aws_lb_target_group.api.arn
    container_name   = "api"
  }
}

resource "aws_ecs_task_definition" "orders" {
  family                   = "${local.app}-orders"
  requires_compatibilities = ["FARGATE"]
  cpu                      = 512
  memory                   = 1024
  container_definitions = jsonencode([
    {
      name  = "orders"
      image = "${aws_ecr_repository.orders.repository_url}:latest"
      environment = [
        { name = "DATABASE_HOST", value = aws_db_instance.main.address },
        { name = "EVENTS_TOPIC", value = aws_sns_topic.order_events.arn },
        { name = "BILLING_URL", value = "http://billing.shipyard.local:7000" },
      ]
    }
  ])
}

resource "aws_ecr_repository" "orders" {
  name = "orders"
}

resource "aws_ecs_service" "orders" {
  name            = "orders"
  cluster         = aws_ecs_cluster.main.id
  task_definition = aws_ecs_task_definition.orders.arn
  desired_count   = 2
  launch_type     = "FARGATE"

  network_configuration {
    subnets = aws_subnet.private[*].id
  }
}

resource "aws_ecs_task_definition" "billing" {
  family                   = "${local.app}-billing"
  requires_compatibilities = ["FARGATE"]
  cpu                      = 512
  memory                   = 1024
  container_definitions = jsonencode([
    {
      name  = "billing"
      image = "${aws_ecr_repository.billing.repository_url}:latest"
      environment = [
        { name = "DATABASE_HOST", value = aws_db_instance.main.address },
        { name = "STRIPE_API_KEY_ARN", value = aws_secretsmanager_secret.stripe.arn },
      ]
    }
  ])
}

resource "aws_ecr_repository" "billing" {
  name = "billing"
}

resource "aws_ecs_service" "billing" {
  name            = "billing"
  cluster         = aws_ecs_cluster.main.id
  task_definition = aws_ecs_task_definition.billing.arn
  desired_count   = 2
  launch_type     = "FARGATE"

  network_configuration {
    subnets = aws_subnet.private[*].id
  }
}

resource "aws_ecs_task_definition" "tracking" {
  family                   = "${local.app}-tracking"
  requires_compatibilities = ["FARGATE"]
  cpu                      = 512
  memory                   = 1024
  container_definitions = jsonencode([
    {
      name  = "tracking"
      image = "${aws_ecr_repository.tracking.repository_url}:latest"
      environment = [
        { name = "SEARCH_HOST", value = aws_opensearch_domain.tracking.endpoint },
        { name = "REDIS_HOST", value = aws_elasticache_replication_group.cache.primary_endpoint_address },
      ]
    }
  ])
}

resource "aws_ecr_repository" "tracking" {
  name = "tracking"
}

resource "aws_ecs_service" "tracking" {
  name            = "tracking"
  cluster         = aws_ecs_cluster.main.id
  task_definition = aws_ecs_task_definition.tracking.arn
  desired_count   = 2
  launch_type     = "FARGATE"

  network_configuration {
    subnets = aws_subnet.private[*].id
  }
}

resource "aws_ecs_task_definition" "notifications" {
  family                   = "${local.app}-notifications"
  requires_compatibilities = ["FARGATE"]
  cpu                      = 512
  memory                   = 1024
  container_definitions = jsonencode([
    {
      name  = "notifications"
      image = "${aws_ecr_repository.notifications.repository_url}:latest"
      environment = [
        { name = "QUEUE_URL", value = aws_sqs_queue.notifications.url },
        { name = "SENDGRID_API_KEY_ARN", value = aws_secretsmanager_secret.sendgrid.arn },
        { name = "TWILIO_AUTH_TOKEN_ARN", value = aws_secretsmanager_secret.twilio.arn },
      ]
    }
  ])
}

resource "aws_ecr_repository" "notifications" {
  name = "notifications"
}

resource "aws_ecs_service" "notifications" {
  name            = "notifications"
  cluster         = aws_ecs_cluster.main.id
  task_definition = aws_ecs_task_definition.notifications.arn
  desired_count   = 2
  launch_type     = "FARGATE"

  network_configuration {
    subnets = aws_subnet.private[*].id
  }
}

resource "aws_ecs_task_definition" "ingest_worker" {
  family                   = "${local.app}-ingest-worker"
  requires_compatibilities = ["FARGATE"]
  cpu                      = 512
  memory                   = 1024
  container_definitions = jsonencode([
    {
      name  = "ingest-worker"
      image = "${aws_ecr_repository.ingest_worker.repository_url}:latest"
      environment = [
        { name = "QUEUE_URL", value = aws_sqs_queue.shipments.url },
        { name = "SEARCH_HOST", value = aws_opensearch_domain.tracking.endpoint },
      ]
    }
  ])
}

resource "aws_ecr_repository" "ingest_worker" {
  name = "ingest-worker"
}

resource "aws_ecs_service" "ingest_worker" {
  name            = "ingest-worker"
  cluster         = aws_ecs_cluster.main.id
  task_definition = aws_ecs_task_definition.ingest_worker.arn
  desired_count   = 2
  launch_type     = "FARGATE"

  network_configuration {
    subnets = aws_subnet.private[*].id
  }
}
