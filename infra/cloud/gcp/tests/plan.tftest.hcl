mock_provider "google" {}
variables {
  project_id      = "piggybank-test-project"
  artifact_bucket = "piggybank-test-artifacts"
  release_object  = "releases/test-01.tgz"
  release_sha256  = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
}
run "private_evaluation_plan" {
  command = plan
  assert {
    condition     = length(google_compute_instance.bank.network_interface[0].access_config) == 0
    error_message = "GCP VM must have no public address."
  }
  assert {
    condition     = google_compute_firewall.iap_ssh.source_ranges == toset(["35.235.240.0/20"]) && google_compute_firewall.iap_ssh.allow == toset([{ protocol = "tcp", ports = tolist(["22"]) }])
    error_message = "Only SSH via IAP may enter the VPC."
  }
  assert {
    condition     = google_compute_instance.bank.deletion_protection && google_compute_disk.data.size >= 100
    error_message = "Retain data and VM protection."
  }
}
