mock_provider "aws" {}
override_data {
  target = data.aws_ami.ubuntu
  values = { id = "ami-0123456789abcdef0" }
}
override_data {
  target = data.aws_ssm_parameter.ubuntu
  values = { value = "ami-0123456789abcdef0" }
}
variables {
  artifact_bucket = "piggybank-test-artifacts"
  release_object  = "releases/test-01.tgz"
  release_sha256  = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
}
run "closed_ingress_plan" {
  command = plan
  assert {
    condition     = length(aws_security_group.bank.ingress) == 0
    error_message = "SSM deployment must expose no ingress."
  }
  assert {
    condition     = aws_ebs_volume.data.encrypted && aws_instance.bank.root_block_device[0].encrypted
    error_message = "Encrypt both disks."
  }
  assert {
    condition     = aws_instance.bank.metadata_options[0].http_tokens == "required" && aws_instance.bank.disable_api_termination
    error_message = "Require IMDSv2 and protect VM termination."
  }
}
