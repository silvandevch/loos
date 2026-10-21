/* losshost — a minimal PE32+ DLL that gives a loaded LoOS PE a way back into
 * the host. The exported function `host_write` is real Windows-ABI code, but
 * it performs no I/O itself: it forwards to a function pointer stored in the
 * exported data symbol `__loos_host_write_hook`. The host harness fills that
 * slot with its own callback after the loader has mapped the DLL, so the
 * imported call lands in host code without any patching of instructions.
 *
 * freestanding: no CRT, no imports of its own. Entry point is DllMain. */
typedef long (*host_write_fn)(int fd, const void *buf, long len);

__declspec(dllexport) host_write_fn __loos_host_write_hook;

__declspec(dllexport) long host_write(int fd, const void *buf, long len)
{
    if (!__loos_host_write_hook || !buf || len < 0) return -1;
    return __loos_host_write_hook(fd, buf, len);
}

__declspec(dllexport) int DllMain(void *module, unsigned reason, void *reserved)
{
    (void)module; (void)reason; (void)reserved;
    return 1;
}
