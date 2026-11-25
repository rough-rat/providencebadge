/* PY32F002Ax5: 20K flash, 3K RAM. Change this together with the chip
   feature in Cargo.toml and CHIP in flash.sh / .cargo/config.toml. */
MEMORY
{
    FLASH : ORIGIN = 0x08000000, LENGTH = 20K
    RAM   : ORIGIN = 0x20000000, LENGTH = 3K
}
