OCI_NAMESPACE="test/test" \
  OCI_ROOT_URL="127.0.0.1:8080" \
  OCI_CROSSMOUNT_NAMESPACE="test/other" \
  OCI_USERNAME="preston@unlockedlabs.org" \
  OCI_PASSWORD="ChangeMe!" \
  OCI_TEST_PULL=1 \
  OCI_TEST_PUSH=1 \
  OCI_TEST_CONTENT_DISCOVERY=1 \
  OCI_TEST_CONTENT_MANAGEMENT=1 \
  OCI_HIDE_SKIPPED_WORKFLOWS=0 \
  OCI_DEBUG=0 \
  OCI_DELETE_MANIFEST_BEFORE_BLOBS=0 \
  ./conformance.test
