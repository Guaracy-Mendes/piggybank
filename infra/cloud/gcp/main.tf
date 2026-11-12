resource "google_project_service" "apis" {
  for_each           = toset(["compute.googleapis.com", "iap.googleapis.com", "oslogin.googleapis.com", "iam.googleapis.com"])
  service            = each.value
  disable_on_destroy = false
}
resource "google_compute_network" "bank" {
  name                    = var.name
  auto_create_subnetworks = false
  depends_on              = [google_project_service.apis]
}
resource "google_compute_subnetwork" "bank" {
  name                     = var.name
  region                   = var.region
  ip_cidr_range            = "10.84.0.0/24"
  network                  = google_compute_network.bank.id
  private_ip_google_access = true
}
resource "google_compute_router" "bank" {
  name    = var.name
  region  = var.region
  network = google_compute_network.bank.id
}
resource "google_compute_router_nat" "bank" {
  name                               = var.name
  router                             = google_compute_router.bank.name
  region                             = var.region
  nat_ip_allocate_option             = "AUTO_ONLY"
  source_subnetwork_ip_ranges_to_nat = "LIST_OF_SUBNETWORKS"
  subnetwork {
    name                    = google_compute_subnetwork.bank.id
    source_ip_ranges_to_nat = ["ALL_IP_RANGES"]
  }
}
resource "google_compute_firewall" "iap_ssh" {
  name          = "${var.name}-iap-ssh"
  network       = google_compute_network.bank.name
  source_ranges = ["35.235.240.0/20"]
  target_tags   = [var.name]
  allow {
    protocol = "tcp"
    ports    = ["22"]
  }
}
resource "google_service_account" "vm" {
  account_id   = var.name
  display_name = "Piggybank evaluation artifact reader"
  depends_on   = [google_project_service.apis]
}
resource "google_storage_bucket_iam_member" "artifacts" {
  bucket = var.artifact_bucket
  role   = "roles/storage.objectViewer"
  member = "serviceAccount:${google_service_account.vm.email}"
  condition {
    title      = "read-release-and-owned-backups"
    expression = "resource.name.startsWith('projects/_/buckets/${var.artifact_bucket}/objects/releases/') || resource.name.startsWith('projects/_/buckets/${var.artifact_bucket}/objects/backups/${var.name}/')"
  }
}
resource "google_storage_bucket_iam_member" "backups" {
  bucket = var.artifact_bucket
  role   = "roles/storage.objectCreator"
  member = "serviceAccount:${google_service_account.vm.email}"
  condition {
    title      = "create-owned-backups"
    expression = "resource.name.startsWith('projects/_/buckets/${var.artifact_bucket}/objects/backups/${var.name}/')"
  }
}
resource "google_compute_disk" "data" {
  name   = "${var.name}-data"
  zone   = var.zone
  type   = "pd-balanced"
  size   = var.data_disk_gb
  labels = { application = "piggybank", profile = "synthetic" }
  lifecycle { prevent_destroy = true }
  depends_on = [google_project_service.apis]
}
resource "google_compute_instance" "bank" {
  name                = var.name
  zone                = var.zone
  machine_type        = var.machine_type
  deletion_protection = var.protect_vm
  tags                = [var.name]
  labels              = { application = "piggybank", profile = "synthetic" }
  boot_disk {
    initialize_params {
      image = "ubuntu-os-cloud/ubuntu-2404-lts-amd64"
      size  = 60
      type  = "pd-balanced"
    }
  }
  attached_disk {
    source      = google_compute_disk.data.id
    device_name = "piggybank-data"
    mode        = "READ_WRITE"
  }
  network_interface {
    subnetwork = google_compute_subnetwork.bank.id
    # No external address; outbound package downloads use Cloud NAT.
  }
  service_account {
    email  = google_service_account.vm.email
    scopes = ["cloud-platform"]
  }
  shielded_instance_config {
    enable_secure_boot          = true
    enable_vtpm                 = true
    enable_integrity_monitoring = true
  }
  metadata = { enable-oslogin = "TRUE", block-project-ssh-keys = "TRUE" }
  metadata_startup_script = templatefile("${path.module}/startup.sh.tftpl", {
    bootstrap      = file("${path.module}/../shared/host-bootstrap.sh")
    bucket         = var.artifact_bucket
    object         = var.release_object
    release_sha256 = var.release_sha256
  })
  depends_on = [google_compute_router_nat.bank, google_storage_bucket_iam_member.artifacts]
}
