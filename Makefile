CC ?= cc
CFLAGS ?= -std=c11 -O2 -g -Wall -Wextra -Werror -pedantic

BTRFS_SRC := src/fs/btrfs/btrfs.c
BTRFS_MKFS_SRC := src/fs/btrfs/mkfs.c
BTRFS_INTEGRITY_SRC := src/fs/btrfs/tools/integrity.c
BTRFS_TEST_SRC := tests/fs/btrfs/test_btrfs.c
BTRFS_INTEGRITY_TEST_SRC := tests/fs/btrfs/test_integrity.c
BTRFS_TEST_BIN := build/test-btrfs
BTRFS_INTEGRITY_TEST_BIN := build/test-btrfs-integrity
BTRFS_CHECK_BIN := build/btrfs-check
BTRFS_FIXTURE := build/btrfs-fixture.img

.PHONY: all test real-test clean

all: $(BTRFS_TEST_BIN) $(BTRFS_INTEGRITY_TEST_BIN) $(BTRFS_CHECK_BIN)

$(BTRFS_TEST_BIN): $(BTRFS_SRC) src/fs/btrfs/btrfs.h $(BTRFS_TEST_SRC)
	@mkdir -p build
	$(CC) $(CFLAGS) -Isrc/fs/btrfs $(BTRFS_SRC) $(BTRFS_TEST_SRC) -o $@

$(BTRFS_INTEGRITY_TEST_BIN): $(BTRFS_SRC) $(BTRFS_MKFS_SRC) $(BTRFS_INTEGRITY_SRC) src/fs/btrfs/btrfs.h src/fs/btrfs/tools/integrity.h $(BTRFS_INTEGRITY_TEST_SRC)
	@mkdir -p build
	$(CC) $(CFLAGS) -Isrc/fs/btrfs -Isrc/fs/btrfs/tools $(BTRFS_SRC) $(BTRFS_MKFS_SRC) $(BTRFS_INTEGRITY_SRC) $(BTRFS_INTEGRITY_TEST_SRC) -o $@

$(BTRFS_CHECK_BIN): $(BTRFS_SRC) $(BTRFS_MKFS_SRC) $(BTRFS_INTEGRITY_SRC) src/fs/btrfs/btrfs.h src/fs/btrfs/tools/integrity.h tools/btrfs_check.c
	@mkdir -p build
	$(CC) $(CFLAGS) -Isrc/fs/btrfs -Isrc/fs/btrfs/tools $(BTRFS_SRC) $(BTRFS_MKFS_SRC) $(BTRFS_INTEGRITY_SRC) tools/btrfs_check.c -o $@

$(BTRFS_FIXTURE): tests/fs/btrfs/make_fixture.py
	@mkdir -p build
	python3 tests/fs/btrfs/make_fixture.py $@

test: $(BTRFS_TEST_BIN) $(BTRFS_INTEGRITY_TEST_BIN) $(BTRFS_FIXTURE)
	$(BTRFS_TEST_BIN) $(BTRFS_FIXTURE) --fixture
	$(BTRFS_INTEGRITY_TEST_BIN)
	@$(MAKE) --no-print-directory real-test

real-test: $(BTRFS_TEST_BIN)
	@python3 tests/fs/btrfs/run_real_test.py $(BTRFS_TEST_BIN); rc=$$?; \
	if [ $$rc -ne 0 ] && [ $$rc -ne 77 ]; then exit $$rc; fi

clean:
	rm -rf build/test-btrfs build/test-btrfs-integrity build/btrfs-check build/btrfs-fixture.img
