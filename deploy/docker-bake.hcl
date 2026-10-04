# All production images in one build; the Rust compile stage is shared by every service.
#   REGISTRY=ghcr.io/<owner> VERSION=v1.2.3 SITE_URL=https://example.com docker buildx bake -f deploy/docker-bake.hcl

variable "REGISTRY" {
  default = "karbird.local"
}

variable "VERSION" {
  default = "dev"
}

variable "SITE_URL" {
  default = "http://localhost:3000"
}

group "default" {
  targets = ["ingest", "processor", "api", "executor", "archiver", "web"]
}

target "_common" {
  context    = "."
  dockerfile = "deploy/Dockerfile"
  platforms  = ["linux/amd64"]
}

target "ingest" {
  inherits = ["_common"]
  target   = "ingest"
  tags     = ["${REGISTRY}/karbird-ingest:${VERSION}"]
}

target "processor" {
  inherits = ["_common"]
  target   = "processor"
  tags     = ["${REGISTRY}/karbird-processor:${VERSION}"]
}

target "api" {
  inherits = ["_common"]
  target   = "api"
  tags     = ["${REGISTRY}/karbird-api:${VERSION}"]
}

target "executor" {
  inherits = ["_common"]
  target   = "executor"
  tags     = ["${REGISTRY}/karbird-executor:${VERSION}"]
}

target "archiver" {
  inherits = ["_common"]
  target   = "archiver"
  tags     = ["${REGISTRY}/karbird-archiver:${VERSION}"]
}

target "web" {
  inherits = ["_common"]
  target   = "web"
  args     = { KARBIRD_SITE_URL = SITE_URL }
  tags     = ["${REGISTRY}/karbird-web:${VERSION}"]
}
