#!/bin/sh
set -e
cd "$(dirname "$0")"
case "$(uname -s)" in
MINGW* | MSYS*)
    export DEVKITPRO=C:/devkitPro
    export DEVKITARM=C:/devkitPro/devkitARM
    export PATH="/c/devkitPro/devkitARM/bin:/c/devkitPro/tools/bin:$PATH"
    LIBCLANG_PATH="$(python -c "import clang, os; print(os.path.join(os.path.dirname(clang.__file__), 'native'))")"
    export LIBCLANG_PATH
    GCC_VERSION="$(arm-none-eabi-gcc -dumpversion)"
    export BINDGEN_EXTRA_CLANG_ARGS="--target=arm-none-eabi -march=armv6k -mfloat-abi=hard -mfpu=vfp --sysroot=C:/devkitPro/devkitARM/arm-none-eabi -isystem C:/devkitPro/devkitARM/arm-none-eabi/include -isystem C:/devkitPro/devkitARM/lib/gcc/arm-none-eabi/$GCC_VERSION/include -IC:/devkitPro/libctru/include -DARM11 -D__3DS__ -fshort-enums"
    ;;
*)
    : "${DEVKITPRO:?Set DEVKITPRO to the devkitPro folder}"
    export DEVKITARM="$DEVKITPRO/devkitARM"
    export PATH="$DEVKITPRO/devkitARM/bin:$DEVKITPRO/tools/bin:$PATH"
    ;;
esac
cargo 3ds build --release "$@"
OUT=target/armv6k-nintendo-3ds/release/kt3-player-3ds
smdhtool --create "KT3 Player for 3DS" "Plays .kt3 files" "keitodaze" icon.png "$OUT.smdh"
3dsxtool "$OUT.elf" "$OUT.3dsx" --smdh="$OUT.smdh"
