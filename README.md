# os81

## Building running debugging

Use cargo aliases: `kbuild`, `kimage`, `krun`, `ktest`.

To debug with qemu, run something like
```
$ qemu-system-x86_64 -drive format=raw,file=target/x86_64-custom/debug/boot-bios-os81.img --no-reboot -device isa-debug-exit,iobase=0xf4,iosize=0x04 -serial stdio -s -S
```
with the -S telling qemu to wait for gdb.
Then run
```
$ gdb ./target/x86_64-custom/debug/os81
...
(gdb) target remote localhost:1234
```

QEMU, and our krun alias by extension, use BIOS by default. To test/verify UEFI
booting, yoiu need the qemu-compatible BIOS firmware OVMF, usually available in
your distros package manager. Then just run, noting the drive arguments and the
-boot flag,

```
qemu-system-x86_64 -drive format=raw,file=target/x86_64-custom/debug/boot-uefi-os81.efi --no-reboot -device isa-debug-exit,iobase=0xf4,iosize=0x04 -serial stdio -s -bios /usr/share/ovmf/OVMF.fd
```
