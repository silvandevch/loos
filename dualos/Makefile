K32 := target/i586-dualos-none/release/kernel32
K64 := target/x86_64-dualos-none/release/kernel64
ISO := out/dualos.iso
QEMU_COMMON := -display none -serial stdio -no-reboot -device isa-debug-exit,iobase=0xf4,iosize=0x04

.PHONY: all kernels iso test clean qemu32 qemu64
all: iso

kernels:
	cargo build --release -p kernel32 --target targets/i586-dualos-none.json
	cargo build --release -p kernel64 --target targets/x86_64-dualos-none.json

iso: kernels
	./scripts/mkiso.sh $(K32) $(K64) $(ISO)

qemu32: iso
	qemu-system-i386 -cpu pentium -m 64 -cdrom $(ISO) $(QEMU_COMMON)
qemu64: iso
	qemu-system-x86_64 -cpu qemu64 -m 64 -cdrom $(ISO) $(QEMU_COMMON)

test: iso
	./scripts/test-boot.sh $(ISO)

clean:
	cargo clean
	rm -rf out
