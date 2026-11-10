"""Print a compiled US XKB keymap (the text wl_keyboard.keymap hands to Wayland clients)."""
import ctypes, sys
x = ctypes.CDLL("libxkbcommon.so.0")
x.xkb_context_new.restype = ctypes.c_void_p
x.xkb_keymap_new_from_names.restype = ctypes.c_void_p
x.xkb_keymap_new_from_names.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_int]
x.xkb_keymap_get_as_string.restype = ctypes.c_void_p
x.xkb_keymap_get_as_string.argtypes = [ctypes.c_void_p, ctypes.c_int]
ctx = x.xkb_context_new(0)
km = x.xkb_keymap_new_from_names(ctx, None, 0) if ctx else None
if not km:
    sys.exit("xkbcommon could not compile the default keymap")
s = ctypes.string_at(x.xkb_keymap_get_as_string(km, 1))
sys.stdout.buffer.write(s + b"\0")
