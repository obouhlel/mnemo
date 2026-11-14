# Mnemo — point d'entrée unique pour développer et compiler.
# `make` (ou `make help`) liste les cibles. Les binaires et paquets sont écrits dans dist/.
#
# Compilation croisée depuis Linux : `make setup` une fois (nécessite rustup, zig et mingw-w64).
# macOS ne se compile que sur un Mac (SDK Apple) : `make macos` sur un Mac, sinon GitHub Actions.

SHELL   := bash
.SHELLFLAGS := -eu -o pipefail -c
.DEFAULT_GOAL := help

APP     := mnemo
VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
DIST    := dist
DB_URL  ?= postgres://postgres:postgres@localhost:5432/vault
HOST_OS := $(shell uname -s)

# Cibles Rust. Le suffixe .2.28 (cargo-zigbuild) fixe la glibc minimale : Debian 10+, Ubuntu 18.04+.
GLIBC         := 2.28
LINUX_ARCHES  := x86_64 aarch64
WINDOWS_TARGET := x86_64-pc-windows-gnu
LINUX_FILES   := packaging/linux/mnemo.desktop assets/icons/mnemo-256.png README.md

##@ Développement

.PHONY: help
help: ## Affiche cette aide
	@awk 'BEGIN {FS = ":.*##"} \
	  /^##@/ { printf "\n\033[1m%s\033[0m\n", substr($$0, 5) } \
	  /^[a-zA-Z_-]+:.*##/ { printf "  \033[36m%-12s\033[0m %s\n", $$1, $$2 }' $(MAKEFILE_LIST)
	@echo

.PHONY: db
db: ## Démarre PostgreSQL 16 dans Docker (localhost:5432, base « vault »)
	docker compose up -d

.PHONY: db-down
db-down: ## Arrête PostgreSQL (les données sont conservées dans le volume Docker)
	docker compose down

.PHONY: run
run: db ## Lance Mnemo en release, connecté à la base locale
	MNEMO_DATABASE_URL="$(DB_URL)" cargo run --release

.PHONY: test
test: ## Lance les tests (avec MNEMO_TEST_PG=postgres://… : aussi la synchronisation à deux appareils)
	cargo test

.PHONY: fmt
fmt: ## Formate le code
	cargo fmt

.PHONY: lint
lint: ## Vérifie le formatage et les avertissements clippy
	cargo fmt -- --check
	cargo clippy --all-targets -- -D warnings

.PHONY: check
check: lint test ## lint + test (à lancer avant chaque commit)

##@ Compilation

.PHONY: setup
setup: ## Installe les cibles Rust et outils de compilation croisée
	@command -v rustup >/dev/null || { echo "rustup manquant. Arch : sudo pacman -S rustup && rustup default stable"; exit 1; }
	rustup target add $(foreach a,$(LINUX_ARCHES),$(a)-unknown-linux-gnu) $(WINDOWS_TARGET)
	cargo install --locked cargo-zigbuild cargo-deb
	@command -v zig >/dev/null || echo "⚠ zig manquant (make linux). Arch : sudo pacman -S zig"
	@command -v x86_64-w64-mingw32-gcc >/dev/null || echo "⚠ mingw-w64 manquant (make windows). Arch : sudo pacman -S mingw-w64-gcc"

ifeq ($(HOST_OS),Darwin)
ALL_BUILDS := macos
else
ALL_BUILDS := linux deb windows
endif

.PHONY: all
all: $(ALL_BUILDS) ## Compile pour tous les OS possibles depuis cette machine

.PHONY: linux
linux: ## Binaires Linux x86_64 + aarch64 (glibc 2.28) → dist/*.tar.gz
	@mkdir -p $(DIST)
	@for arch in $(LINUX_ARCHES); do \
	  cargo zigbuild --release --target $$arch-unknown-linux-gnu.$(GLIBC); \
	  dir=$(DIST)/$(APP)-$(VERSION)-linux-$$arch; rm -rf $$dir; mkdir -p $$dir; \
	  cp target/$$arch-unknown-linux-gnu/release/$(APP) $(LINUX_FILES) $$dir/; \
	  tar czf $$dir.tar.gz -C $(DIST) $$(basename $$dir); rm -rf $$dir; \
	  echo "→ $$dir.tar.gz"; \
	done

.PHONY: deb
deb: ## Paquet Debian / Ubuntu (x86_64, glibc 2.28) → dist/*.deb
	cargo zigbuild --release --target x86_64-unknown-linux-gnu.$(GLIBC)
	@mkdir -p $(DIST)
	cargo deb --no-build --no-strip --target x86_64-unknown-linux-gnu -o $(DIST)/

.PHONY: arch
arch: ## Paquet Arch Linux → dist/*.pkg.tar.zst (installer : sudo pacman -U dist/…)
	@mkdir -p $(DIST)
	cd packaging/arch && PKGDEST="$(abspath $(DIST))" makepkg -f --cleanbuild
	rm -rf packaging/arch/src packaging/arch/pkg

.PHONY: windows
windows: ## Exécutable Windows (x86_64) → dist/mnemo.exe
	@mkdir -p $(DIST)
ifeq ($(OS),Windows_NT)
	cargo build --release
	cp target/release/$(APP).exe $(DIST)/$(APP).exe
else
	cargo build --release --target $(WINDOWS_TARGET)
	cp target/$(WINDOWS_TARGET)/release/$(APP).exe $(DIST)/$(APP).exe
endif
	@echo "→ $(DIST)/$(APP).exe"

.PHONY: macos
macos: ## Mnemo.app universelle + .dmg (uniquement sur un Mac) → dist/
ifeq ($(HOST_OS),Darwin)
	./scripts/build-macos.sh
else
	@echo "macOS ne peut être compilé que sur un Mac (SDK Apple)."
	@echo "Sinon : GitHub › Actions › build › Run workflow, puis téléchargez l'artefact mnemo-macos."
	@exit 1
endif

.PHONY: clean
clean: ## Supprime target/ et dist/
	cargo clean
	rm -rf $(DIST)
