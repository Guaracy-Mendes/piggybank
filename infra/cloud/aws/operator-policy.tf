data "aws_caller_identity" "current" {}

output "operator_policy_json" {
  description = "Attach to the designated operations role, separately from the VM role."
  value = jsonencode({
    Version = "2012-10-17"
    Statement = [
      { Effect = "Allow", Action = ["ec2:DescribeInstances", "ssm:DescribeInstanceInformation", "ssm:GetCommandInvocation", "ssm:ListCommands", "ssm:DescribeSessions"], Resource = "*" },
      { Effect = "Allow", Action = ["ssm:StartSession", "ssm:SendCommand"], Resource = [aws_instance.bank.arn, "arn:aws:ssm:${var.region}::document/AWS-StartPortForwardingSession", "arn:aws:ssm:${var.region}:${data.aws_caller_identity.current.account_id}:document/SSM-SessionManagerRunShell", "arn:aws:ssm:${var.region}::document/AWS-RunShellScript"] },
      { Effect = "Allow", Action = ["ssm:TerminateSession", "ssm:ResumeSession"], Resource = "arn:aws:ssm:${var.region}:*:session/$${aws:userid}-*" },
      { Effect = "Allow", Action = ["ssmmessages:OpenDataChannel"], Resource = "arn:aws:ssm:${var.region}:*:session/$${aws:userid}-*" }
    ]
  })
}
