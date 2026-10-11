/* A generic small MCU: 256 KiB of flash and 64 KiB of RAM, at the addresses
   most Cortex-M vendors use. The image only has to link, so no particular
   chip is meant, and the RISC-V image uses the same layout. The runtimes'
   link.x scripts include this file. */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 256K
  RAM   : ORIGIN = 0x20000000, LENGTH = 64K
}

/* riscv-rt places its sections through these aliases; cortex-m-rt uses
   FLASH and RAM directly and ignores them. */
REGION_ALIAS("REGION_TEXT", FLASH);
REGION_ALIAS("REGION_RODATA", FLASH);
REGION_ALIAS("REGION_DATA", RAM);
REGION_ALIAS("REGION_BSS", RAM);
REGION_ALIAS("REGION_HEAP", RAM);
REGION_ALIAS("REGION_STACK", RAM);
