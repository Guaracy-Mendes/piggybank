output "instance" { value = google_compute_instance.bank.name }
output "zone" { value = var.zone }
output "service_account" { value = google_service_account.vm.email }
output "data_disk" { value = google_compute_disk.data.name }
output "private_ip" { value = google_compute_instance.bank.network_interface[0].network_ip }
