output "instance_id" { value = aws_instance.bank.id }
output "region" { value = var.region }
output "ami_id" { value = aws_instance.bank.ami }
output "data_volume_id" { value = aws_ebs_volume.data.id }
output "instance_role" { value = aws_iam_role.vm.name }
