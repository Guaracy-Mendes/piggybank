variable "project_id" { type = string }
variable "region" {
  type    = string
  default = "us-central1"
}
variable "zone" {
  type    = string
  default = "us-central1-a"
}
variable "name" {
  type    = string
  default = "piggybank-eval"
  validation {
    condition     = can(regex("^[a-z][a-z0-9-]{2,23}$", var.name))
    error_message = "Use a 3-24 character lowercase resource prefix."
  }
}
variable "machine_type" {
  type    = string
  default = "e2-standard-4"
}
variable "data_disk_gb" {
  type    = number
  default = 100
  validation {
    condition     = var.data_disk_gb >= 100
    error_message = "Use at least 100 GiB for data and stopped backups."
  }
}
variable "artifact_bucket" { type = string }
variable "release_object" {
  type = string
  validation {
    condition     = can(regex("^releases/[a-z0-9][a-z0-9.-]*[.]tgz$", var.release_object))
    error_message = "Use releases/<immutable-release>.tgz."
  }
}
variable "release_sha256" {
  type = string
  validation {
    condition     = can(regex("^[a-f0-9]{64}$", var.release_sha256))
    error_message = "Supply the exact release SHA256."
  }
}
variable "protect_vm" {
  type    = bool
  default = true
}
