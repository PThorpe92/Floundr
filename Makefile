DOCKERFILE_TEMPLATE = Dockerfile.template
DOCKERFILE = Dockerfile
DOCKER_IMAGE = harbor
DOCKER_TAG = latest
DATABASE_URL ?= $(DATABASE_URL)

.PHONY: build clean help run

help:
	@echo 'Usage: make [target] DATABASE_URL=<path>'
	@echo 'Targets:'
	@echo '  build: Build the Docker image'
	@echo '  clean: Clean up the Dockerfile'
  @echo '  run: Run the docker compose containers'
	@echo '  help: Show this help message'

build:
	@echo "Creating Dockerfile from template..."
	@sed -e 's|DB_URL|$(DATABASE_DIR)|g' $(DOCKERFILE_TEMPLATE) > $(DOCKERFILE)
	@echo "Building Docker image..."
	@docker build -t $(DOCKER_IMAGE):$(DOCKER_TAG) .

clean:
	@echo "Cleaning up..."
	@rm -f $(DOCKERFILE)

run:
	@echo "Running docker compose..."
	@docker-compose up
