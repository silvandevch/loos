/* The subset of the UEFI 2.x interfaces the LoOS loader needs, written from the UEFI specification
 * (https://uefi.org/specifications): table layouts, GUID values and function order are spec-defined facts.
 * Built with the x86_64 Windows target, whose default calling convention is the UEFI one. */
#ifndef LOOS_EFI_H
#define LOOS_EFI_H
#include <stddef.h>
#include <stdint.h>

typedef uint64_t UINTN;
typedef UINTN EFI_STATUS;
typedef void *EFI_HANDLE;
typedef uint16_t CHAR16;
typedef uint64_t EFI_PHYSICAL_ADDRESS;

#define EFI_ERROR(s)  ((int64_t)(s) < 0)
#define EFI_SUCCESS   0ull
#define EFI_BUFFER_TOO_SMALL (0x8000000000000005ull)
#define EFI_INVALID_PARAMETER (0x8000000000000002ull)

typedef struct { uint32_t a; uint16_t b, c; uint8_t d[8]; } EFI_GUID;

typedef struct { uint64_t Signature; uint32_t Revision, HeaderSize, CRC32, Reserved; } EFI_TABLE_HEADER;

typedef struct EFI_SIMPLE_TEXT_OUTPUT {
    void *Reset;
    EFI_STATUS (*OutputString)(struct EFI_SIMPLE_TEXT_OUTPUT *, const CHAR16 *);
} EFI_SIMPLE_TEXT_OUTPUT;

typedef struct { uint32_t Type; uint32_t Pad; uint64_t PhysicalStart, VirtualStart, NumberOfPages, Attribute; } EFI_MEMORY_DESCRIPTOR;

enum { AllocateAnyPages = 0, AllocateMaxAddress = 1, AllocateAddress = 2 };
enum { EfiReservedMemoryType, EfiLoaderCode, EfiLoaderData, EfiBootServicesCode, EfiBootServicesData, EfiRuntimeServicesCode,
       EfiRuntimeServicesData, EfiConventionalMemory, EfiUnusableMemory, EfiACPIReclaimMemory, EfiACPIMemoryNVS,
       EfiMemoryMappedIO, EfiMemoryMappedIOPortSpace, EfiPalCode, EfiPersistentMemory };

typedef struct {
    EFI_TABLE_HEADER Hdr;
    void *RaiseTPL, *RestoreTPL;
    EFI_STATUS (*AllocatePages)(int type, int memtype, UINTN pages, EFI_PHYSICAL_ADDRESS *addr);
    EFI_STATUS (*FreePages)(EFI_PHYSICAL_ADDRESS addr, UINTN pages);
    EFI_STATUS (*GetMemoryMap)(UINTN *size, EFI_MEMORY_DESCRIPTOR *map, UINTN *key, UINTN *desc_size, uint32_t *desc_ver);
    void *AllocatePool, *FreePool, *CreateEvent, *SetTimer, *WaitForEvent, *SignalEvent, *CloseEvent, *CheckEvent;
    void *InstallProtocolInterface, *ReinstallProtocolInterface, *UninstallProtocolInterface;
    EFI_STATUS (*HandleProtocol)(EFI_HANDLE, const EFI_GUID *, void **);
    void *Reserved, *RegisterProtocolNotify, *LocateHandle, *LocateDevicePath, *InstallConfigurationTable;
    void *LoadImage, *StartImage, *Exit, *UnloadImage;
    EFI_STATUS (*ExitBootServices)(EFI_HANDLE, UINTN key);
    void *GetNextMonotonicCount;
    EFI_STATUS (*Stall)(UINTN us);
    EFI_STATUS (*SetWatchdogTimer)(UINTN timeout, uint64_t code, UINTN datasize, const CHAR16 *data);
    void *ConnectController, *DisconnectController, *OpenProtocol, *CloseProtocol, *OpenProtocolInformation, *ProtocolsPerHandle,
         *LocateHandleBuffer;
    EFI_STATUS (*LocateProtocol)(const EFI_GUID *, void *registration, void **iface);
    void *InstallMultipleProtocolInterfaces, *UninstallMultipleProtocolInterfaces, *CalculateCrc32, *CopyMem, *SetMem, *CreateEventEx;
} EFI_BOOT_SERVICES;

typedef struct { EFI_GUID VendorGuid; void *VendorTable; } EFI_CONFIGURATION_TABLE;

typedef struct {
    EFI_TABLE_HEADER Hdr;
    CHAR16 *FirmwareVendor;
    uint32_t FirmwareRevision;
    EFI_HANDLE ConsoleInHandle;
    void *ConIn;
    EFI_HANDLE ConsoleOutHandle;
    EFI_SIMPLE_TEXT_OUTPUT *ConOut;
    EFI_HANDLE StandardErrorHandle;
    EFI_SIMPLE_TEXT_OUTPUT *StdErr;
    void *RuntimeServices;
    EFI_BOOT_SERVICES *BootServices;
    UINTN NumberOfTableEntries;
    EFI_CONFIGURATION_TABLE *ConfigurationTable;
} EFI_SYSTEM_TABLE;

typedef struct {
    uint32_t Revision;
    EFI_HANDLE ParentHandle;
    EFI_SYSTEM_TABLE *SystemTable;
    EFI_HANDLE DeviceHandle;
    void *FilePath, *Reserved;
    uint32_t LoadOptionsSize;
    void *LoadOptions;
    void *ImageBase;
    uint64_t ImageSize;
    int ImageCodeType, ImageDataType;
    void *Unload;
} EFI_LOADED_IMAGE_PROTOCOL;

typedef struct EFI_FILE_PROTOCOL {
    uint64_t Revision;
    EFI_STATUS (*Open)(struct EFI_FILE_PROTOCOL *, struct EFI_FILE_PROTOCOL **, const CHAR16 *name, uint64_t mode, uint64_t attrs);
    EFI_STATUS (*Close)(struct EFI_FILE_PROTOCOL *);
    void *Delete;
    EFI_STATUS (*Read)(struct EFI_FILE_PROTOCOL *, UINTN *size, void *buf);
    void *Write;
    EFI_STATUS (*GetPosition)(struct EFI_FILE_PROTOCOL *, uint64_t *);
    EFI_STATUS (*SetPosition)(struct EFI_FILE_PROTOCOL *, uint64_t);
    void *GetInfo, *SetInfo, *Flush;
} EFI_FILE_PROTOCOL;
#define EFI_FILE_MODE_READ 1ull

typedef struct EFI_SIMPLE_FILE_SYSTEM_PROTOCOL {
    uint64_t Revision;
    EFI_STATUS (*OpenVolume)(struct EFI_SIMPLE_FILE_SYSTEM_PROTOCOL *, EFI_FILE_PROTOCOL **root);
} EFI_SIMPLE_FILE_SYSTEM_PROTOCOL;

typedef struct {
    uint32_t Version, HorizontalResolution, VerticalResolution, PixelFormat;
    uint32_t PixelInformation[4];
    uint32_t PixelsPerScanLine;
} EFI_GRAPHICS_OUTPUT_MODE_INFORMATION;
enum { PixelRedGreenBlueReserved8BitPerColor = 0, PixelBlueGreenRedReserved8BitPerColor = 1, PixelBitMask = 2, PixelBltOnly = 3 };

typedef struct EFI_GRAPHICS_OUTPUT_PROTOCOL {
    EFI_STATUS (*QueryMode)(struct EFI_GRAPHICS_OUTPUT_PROTOCOL *, uint32_t, UINTN *, EFI_GRAPHICS_OUTPUT_MODE_INFORMATION **);
    EFI_STATUS (*SetMode)(struct EFI_GRAPHICS_OUTPUT_PROTOCOL *, uint32_t);
    void *Blt;
    struct {
        uint32_t MaxMode, Mode;
        EFI_GRAPHICS_OUTPUT_MODE_INFORMATION *Info;
        UINTN SizeOfInfo;
        EFI_PHYSICAL_ADDRESS FrameBufferBase;
        UINTN FrameBufferSize;
    } *Mode;
} EFI_GRAPHICS_OUTPUT_PROTOCOL;

/* EFI_PARTITION_INFO_PROTOCOL (UEFI 2.7): tells the loader which GPT partition it was started from */
typedef struct {
    uint32_t Revision, Type;                         /* Type 2 = GPT */
    uint8_t System, Reserved[7];
    union {
        uint8_t Mbr[128];
        struct { EFI_GUID PartitionTypeGUID, UniquePartitionGUID; uint64_t StartingLBA, EndingLBA, Attributes; CHAR16 PartitionName[36]; } Gpt;
    } Info;
} EFI_PARTITION_INFO_PROTOCOL;
#define EFI_PARTITION_INFO_GUID { 0x8cf2f62c, 0xbc9b, 0x4821, { 0x80, 0x8d, 0xec, 0x9e, 0xc4, 0x21, 0xa1, 0xa0 } }

#define EFI_LOADED_IMAGE_GUID { 0x5b1b31a1, 0x9562, 0x11d2, { 0x8e, 0x3f, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b } }
#define EFI_SIMPLE_FS_GUID    { 0x964e5b22, 0x6459, 0x11d2, { 0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b } }
#define EFI_GOP_GUID          { 0x9042a9de, 0x23dc, 0x4a38, { 0x96, 0xfb, 0x7a, 0xde, 0xd0, 0x80, 0x51, 0x6a } }
#define EFI_ACPI20_GUID       { 0x8868e871, 0xe4f1, 0x11d3, { 0xbc, 0x22, 0x00, 0x80, 0xc7, 0x3c, 0x81, 0x88 } }
#define EFI_ACPI10_GUID       { 0xeb9d2d30, 0x2d88, 0x11d3, { 0x9a, 0x16, 0x00, 0x90, 0x27, 0x3f, 0xc1, 0x4d } }
#endif
