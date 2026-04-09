# Canopy — local semantic code search

# --------------------------------------------------------------------------
# Build & Install
# --------------------------------------------------------------------------

# Build canopy
build:
    cargo build

# Install canopy binary
install:
    cargo install --path .

# Run tests
test:
    cargo test

# --------------------------------------------------------------------------
# Canopy Commands
# --------------------------------------------------------------------------

# Show canopy status for current repo
status:
    canopy status

# Full re-index of current repo
reindex:
    canopy reindex

# Incremental index
index:
    canopy index

# Clean canopy from current repo
clean:
    canopy clean

# Nuke and rebuild: clean + init + reindex
reset:
    -canopy clean
    canopy init
    canopy reindex
