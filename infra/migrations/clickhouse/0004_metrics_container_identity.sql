-- What a container runs (ADR-0018, resource placement): its image (`container.image.name`) and
-- its Docker Compose service name (label `com.docker.compose.service`, the DNS name other
-- containers use to reach it). Used to place databases and queues on the machine they run on.

ALTER TABLE metrics ADD COLUMN IF NOT EXISTS container_image LowCardinality(String) DEFAULT '' AFTER container_hostname;

ALTER TABLE metrics ADD COLUMN IF NOT EXISTS container_service LowCardinality(String) DEFAULT '' AFTER container_image;
