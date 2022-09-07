mkfile_path := $(abspath $(lastword $(MAKEFILE_LIST)))

images:
	cargo build
	$(eval $@_PKG := $(shell cargo metadata --format-version 1 | jq '.resolve.nodes | .[] | .deps | .[] | select(.name == "bootloader") | .pkg'))
	@echo $($@_PKG)
	cd $$(cargo metadata --format-version 1 | jq '.packages | .[] | select(.id == $($@_PKG)) | .manifest_path' | xargs dirname) && cargo builder
