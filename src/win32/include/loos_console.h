/* LoOS console device contract (kernel <-> kernel32).
 *
 * A console is a device object. A process gets console handles as ordinary file handles (inherited
 * standard handles, or opened by name: \??\CONIN$ and \??\CONOUT$). The data path is plain file I/O:
 *   NtWriteFile  to an output handle: UTF-8 text, interpreted by the console (VT sequences when the
 *                output mode has ENABLE_VIRTUAL_TERMINAL_PROCESSING, '\n' handling per mode).
 *   NtReadFile   from an input handle: UTF-8 bytes. In line mode (ENABLE_LINE_INPUT) one call returns
 *                at most one edited line ending in "\r\n"; otherwise whatever is typed.
 * Everything else is NtDeviceIoControlFile with the buffered codes below (device type 0x50 =
 * FILE_DEVICE_CONSOLE). Input and output handles to the same console share state (mode is per handle
 * direction; code pages, title and screen state are per console). */
#ifndef LOOS_CONSOLE_H
#define LOOS_CONSOLE_H
#include "loos_nt.h"

#define FILE_DEVICE_CONSOLE 0x50u
#define LOOS_CON_CTL(fn) ((FILE_DEVICE_CONSOLE << 16) | ((fn) << 2))     /* METHOD_BUFFERED, FILE_ANY_ACCESS */

#define IOCTL_CON_GET_MODE        LOOS_CON_CTL(0x100)   /* out: ULONG mode */
#define IOCTL_CON_SET_MODE        LOOS_CON_CTL(0x101)   /* in:  ULONG mode */
#define IOCTL_CON_GET_INFO        LOOS_CON_CTL(0x102)   /* out: LOOS_CON_SCREEN_INFO (output handle) */
#define IOCTL_CON_SET_CURSOR      LOOS_CON_CTL(0x103)   /* in:  SHORT x, SHORT y */
#define IOCTL_CON_SET_ATTR        LOOS_CON_CTL(0x104)   /* in:  USHORT attributes for subsequent text */
#define IOCTL_CON_GET_TITLE       LOOS_CON_CTL(0x105)   /* out: UTF-8, no terminator; IO_STATUS.Information = bytes */
#define IOCTL_CON_SET_TITLE       LOOS_CON_CTL(0x106)   /* in:  UTF-8, no terminator */
#define IOCTL_CON_GET_CP          LOOS_CON_CTL(0x107)   /* in: ULONG is_output (0 input / 1 output); out: ULONG code page */
#define IOCTL_CON_SET_CP          LOOS_CON_CTL(0x108)   /* in: LOOS_CON_SET_CP */
#define IOCTL_CON_FLUSH_INPUT     LOOS_CON_CTL(0x109)   /* discard pending input */
#define IOCTL_CON_INPUT_COUNT     LOOS_CON_CTL(0x10A)   /* out: ULONG bytes available */
#define IOCTL_CON_FILL            LOOS_CON_CTL(0x10B)   /* in: LOOS_CON_FILL */

typedef struct { int16_t x, y; } LOOS_CON_COORD;
typedef struct {
    LOOS_CON_COORD size;               /* buffer size in cells */
    LOOS_CON_COORD cursor;
    uint16_t attributes;
    int16_t window_left, window_top, window_right, window_bottom;
    LOOS_CON_COORD max_window_size;
} LOOS_CON_SCREEN_INFO;                /* the same fields, in the same order, as CONSOLE_SCREEN_BUFFER_INFO */

typedef struct { ULONG is_output; ULONG code_page; } LOOS_CON_SET_CP;
typedef struct { ULONG kind; ULONG value; ULONG count; LOOS_CON_COORD at; } LOOS_CON_FILL;   /* kind 0 = character, 1 = attribute */

/* console mode bits (same values as Win32) */
#define ENABLE_PROCESSED_INPUT 0x0001
#define ENABLE_LINE_INPUT      0x0002
#define ENABLE_ECHO_INPUT      0x0004
#define ENABLE_PROCESSED_OUTPUT 0x0001
#define ENABLE_WRAP_AT_EOL_OUTPUT 0x0002
#define ENABLE_VIRTUAL_TERMINAL_PROCESSING 0x0004

#endif
