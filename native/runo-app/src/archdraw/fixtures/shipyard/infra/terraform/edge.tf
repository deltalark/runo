resource "aws_cloudfront_distribution" "web" {
  enabled = true

  origin {
    domain_name = aws_lb.public.dns_name
    origin_id   = "alb"
  }

  origin {
    domain_name = aws_s3_bucket.assets.bucket_regional_domain_name
    origin_id   = "assets"
  }
}

resource "aws_lb" "public" {
  name               = "${local.app}-public"
  load_balancer_type = "application"
  subnets            = aws_subnet.public[*].id
}

resource "aws_lb_target_group" "web" {
  name   = "${local.app}-web"
  port   = 3000
  vpc_id = aws_vpc.main.id
}

resource "aws_lb_target_group" "api" {
  name   = "${local.app}-api"
  port   = 8080
  vpc_id = aws_vpc.main.id
}

resource "aws_lb_listener" "https" {
  load_balancer_arn = aws_lb.public.arn
  port              = 443
  protocol          = "HTTPS"

  default_action {
    type             = "forward"
    target_group_arn = aws_lb_target_group.web.arn
  }
}

resource "aws_lb_listener_rule" "api" {
  listener_arn = aws_lb_listener.https.arn

  action {
    type             = "forward"
    target_group_arn = aws_lb_target_group.api.arn
  }

  condition {
    path_pattern {
      values = ["/api/*"]
    }
  }
}
