data "aws_ssm_parameter" "ubuntu" {
  name = "/aws/service/canonical/ubuntu/server/24.04/stable/current/amd64/hvm/ebs-gp3/ami-id"
}
data "aws_ami" "ubuntu" {
  owners = ["099720109477"]
  filter {
    name   = "image-id"
    values = [coalesce(var.ami_id, data.aws_ssm_parameter.ubuntu.value)]
  }
  filter {
    name   = "architecture"
    values = ["x86_64"]
  }
}
resource "aws_vpc" "bank" {
  cidr_block           = "10.85.0.0/16"
  enable_dns_support   = true
  enable_dns_hostnames = true
  tags                 = { Name = var.name }
}
resource "aws_subnet" "bank" {
  vpc_id                  = aws_vpc.bank.id
  availability_zone       = var.availability_zone
  cidr_block              = "10.85.1.0/24"
  map_public_ip_on_launch = true
  tags                    = { Name = var.name }
}
resource "aws_internet_gateway" "bank" { vpc_id = aws_vpc.bank.id }
resource "aws_route_table" "bank" {
  vpc_id = aws_vpc.bank.id
  route {
    cidr_block = "0.0.0.0/0"
    gateway_id = aws_internet_gateway.bank.id
  }
}
resource "aws_route_table_association" "bank" {
  subnet_id      = aws_subnet.bank.id
  route_table_id = aws_route_table.bank.id
}
resource "aws_security_group" "bank" {
  name        = var.name
  description = "No inbound rules; SSM uses outbound HTTPS"
  vpc_id      = aws_vpc.bank.id
  ingress     = []
  # No SSH key and no ingress, including from the VPC itself.
  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }
}
resource "aws_iam_role" "vm" {
  name = "${var.name}-vm"
  assume_role_policy = jsonencode({
    Version   = "2012-10-17"
    Statement = [{ Effect = "Allow", Principal = { Service = "ec2.amazonaws.com" }, Action = "sts:AssumeRole" }]
  })
}
resource "aws_iam_role_policy_attachment" "ssm" {
  role       = aws_iam_role.vm.name
  policy_arn = "arn:aws:iam::aws:policy/AmazonSSMManagedInstanceCore"
}
resource "aws_iam_role_policy" "artifacts" {
  name = "release-read-only"
  role = aws_iam_role.vm.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      { Effect = "Allow", Action = ["s3:GetObject"], Resource = ["arn:aws:s3:::${var.artifact_bucket}/releases/*", "arn:aws:s3:::${var.artifact_bucket}/backups/${var.name}/*"] },
      { Effect = "Allow", Action = ["s3:PutObject", "s3:AbortMultipartUpload", "s3:ListMultipartUploadParts"], Resource = "arn:aws:s3:::${var.artifact_bucket}/backups/${var.name}/*" }
    ]
  })
}
resource "aws_iam_instance_profile" "vm" {
  name = var.name
  role = aws_iam_role.vm.name
}
resource "aws_ebs_volume" "data" {
  availability_zone = var.availability_zone
  size              = var.data_disk_gb
  type              = "gp3"
  encrypted         = true
  tags              = { Name = "${var.name}-data" }
  lifecycle { prevent_destroy = true }
}
resource "aws_instance" "bank" {
  ami                         = data.aws_ami.ubuntu.id
  instance_type               = var.instance_type
  subnet_id                   = aws_subnet.bank.id
  vpc_security_group_ids      = [aws_security_group.bank.id]
  associate_public_ip_address = true
  iam_instance_profile        = aws_iam_instance_profile.vm.name
  disable_api_termination     = var.protect_vm
  user_data_replace_on_change = false
  metadata_options {
    http_endpoint               = "enabled"
    http_tokens                 = "required"
    http_put_response_hop_limit = 1
  }
  root_block_device {
    volume_size = 60
    volume_type = "gp3"
    encrypted   = true
  }
  user_data = templatefile("${path.module}/startup.sh.tftpl", {
    bootstrap      = file("${path.module}/../shared/host-bootstrap.sh")
    volume_id      = replace(aws_ebs_volume.data.id, "-", "")
    bucket         = var.artifact_bucket
    object         = var.release_object
    region         = var.region
    release_sha256 = var.release_sha256
  })
  tags       = { Name = var.name }
  depends_on = [aws_route_table_association.bank, aws_iam_role_policy.artifacts, aws_iam_role_policy_attachment.ssm]
}
resource "aws_volume_attachment" "data" {
  device_name = "/dev/sdf"
  volume_id   = aws_ebs_volume.data.id
  instance_id = aws_instance.bank.id
}
