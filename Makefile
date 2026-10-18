CC ?= cc
CFLAGS ?= -std=c11 -O2 -g -Wall -Wextra -Werror -pedantic

BTRFS_SRC := src/fs/btrfs/btrfs.c
BTRFS_TEST_SRC := tests/fs/btrfs/test_btrfs.c
BTRFS_TEST_BIN := build/test-btrfs
BTRFS_FIXTURE := build/btrfs-fixture.img

.PHONY: all test real-test clean

all: $(BTRFS_TEST_BIN)

$(BTRFS_TEST_BIN): $(BTRFS_SRC) src/fs/btrfs/btrfs.h $(BTRFS_TEST_SRC)
	@mkdir -p build
	$(CC) $(CFLAGS) -Isrc/fs/btrfs $(BTRFS_SRC) $(BTRFS_TEST_SRC) -o $@

$(BTRFS_FIXTURE): tests/fs/btrfs/make_fixture.py
	@mkdir -p build
	python3 tests/fs/btrfs/make_fixture.py $@

test: $(BTRFS_TEST_BIN) $(BTRFS_FIXTURE)
	$(BTRFS_TEST_BIN) $(BTRFS_FIXTURE) --fixture
	@$(MAKE) --no-print-directory real-test

real-test: $(BTRFS_TEST_BIN)
	@python3 tests/fs/btrfs/run_real_test.py $(BTRFS_TEST_BIN); rc=$$?; \
	if [ $$rc -ne 0 ] && [ $$rc -ne 77 ]; then exit $$rc; fi

clean:
	rm -rf build/test-btrfs build/btrfs-fixture.img
