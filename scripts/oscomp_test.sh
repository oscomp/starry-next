#!/bin/bash

TIMEOUT=600s
EXIT_STATUS=0
ROOT=$(realpath $(dirname $0))/../
AX_ROOT=$ROOT/.arceos
S_PASS=0
S_FAILED=1
S_TIMEOUT=2
S_BUILD_FAILED=3

RED_C="\x1b[31;1m"
GREEN_C="\x1b[32;1m"
YELLOW_C="\x1b[33;1m"
CYAN_C="\x1b[36;1m"
BLOD_C="\x1b[1m"
END_C="\x1b[0m"

if [ -z "$ARCH" ]; then
    ARCH=x86_64
fi
if [ "$ARCH" != "x86_64" ] && [ "$ARCH" != "riscv64" ] && [ "$ARCH" != "aarch64" ] && [ "$ARCH" != "loongarch64" ]; then
    echo "Unknown architecture: $ARCH"
    exit $S_FAILED
fi

if [ -z "$LIBC" ]; then
    LIBC=musl
fi
if [ "$LIBC" != "musl" ] && [ "$LIBC" != "glibc" ]; then
    echo "Unknown libc: $LIBC"
    exit $S_FAILED
fi

# TODO: add more basic testcases
basic_testlist=(
    "/$LIBC/basic/brk"
    "/$LIBC/basic/chdir"
    "/$LIBC/basic/clone"
    "/$LIBC/basic/close"
    "/$LIBC/basic/dup2"
    "/$LIBC/basic/dup"
    "/$LIBC/basic/execve"
    "/$LIBC/basic/exit"
    "/$LIBC/basic/fork"
    "/$LIBC/basic/fstat"
    "/$LIBC/basic/getcwd"
    "/$LIBC/basic/getdents"
    "/$LIBC/basic/getpid"
    "/$LIBC/basic/getppid"
    "/$LIBC/basic/gettimeofday"
    "/$LIBC/basic/mkdir_"
    "/$LIBC/basic/mmap"
    "/$LIBC/basic/mount"
    "/$LIBC/basic/munmap"
    "/$LIBC/basic/openat"
    "/$LIBC/basic/open"
    "/$LIBC/basic/pipe"
    "/$LIBC/basic/read"
    "/$LIBC/basic/sleep"
    "/$LIBC/basic/times"
    "/$LIBC/basic/umount"
    "/$LIBC/basic/uname"
    "/$LIBC/basic/unlink"
    "/$LIBC/basic/wait"
    "/$LIBC/basic/waitpid"
    "/$LIBC/basic/write"
    "/$LIBC/basic/yield"
)
busybox_testlist=("/$LIBC/busybox sh /$LIBC/busybox_testcode.sh")
iozone_testlist=("/$LIBC/busybox sh /$LIBC/iozone_testcode.sh")
lua_testlist=("/$LIBC/busybox sh /$LIBC/lua_testcode.sh")
libctest_testlist=(
    "/$LIBC/runtest.exe -w entry-static.exe argv"
    "/$LIBC/runtest.exe -w entry-static.exe basename"
    "/$LIBC/runtest.exe -w entry-static.exe dirname"
    "/$LIBC/runtest.exe -w entry-static.exe env"
    "/$LIBC/runtest.exe -w entry-static.exe fdopen"
    "/$LIBC/runtest.exe -w entry-static.exe inet_pton"
    "/$LIBC/runtest.exe -w entry-static.exe memstream"
    "/$LIBC/runtest.exe -w entry-static.exe random"
    "/$LIBC/runtest.exe -w entry-static.exe search_hsearch"
    "/$LIBC/runtest.exe -w entry-static.exe search_insque"
    "/$LIBC/runtest.exe -w entry-static.exe search_lsearch"
    "/$LIBC/runtest.exe -w entry-static.exe search_tsearch"
    "/$LIBC/runtest.exe -w entry-static.exe snprintf"
    "/$LIBC/runtest.exe -w entry-static.exe string"
    "/$LIBC/runtest.exe -w entry-static.exe string_memcpy"
    "/$LIBC/runtest.exe -w entry-static.exe string_memmem"
    "/$LIBC/runtest.exe -w entry-static.exe string_memset"
    "/$LIBC/runtest.exe -w entry-static.exe string_strchr"
    "/$LIBC/runtest.exe -w entry-static.exe string_strcspn"
    "/$LIBC/runtest.exe -w entry-static.exe string_strstr"
    "/$LIBC/runtest.exe -w entry-static.exe strptime"
    "/$LIBC/runtest.exe -w entry-static.exe strtod"
    "/$LIBC/runtest.exe -w entry-static.exe strtod_simple"
    "/$LIBC/runtest.exe -w entry-static.exe strtof"
    "/$LIBC/runtest.exe -w entry-static.exe strtold"
    "/$LIBC/runtest.exe -w entry-static.exe tgmath"
    "/$LIBC/runtest.exe -w entry-static.exe time"
    "/$LIBC/runtest.exe -w entry-static.exe tls_align"
    "/$LIBC/runtest.exe -w entry-static.exe udiv"
    "/$LIBC/runtest.exe -w entry-static.exe wcsstr"
    "/$LIBC/runtest.exe -w entry-static.exe fgets_eof"
    "/$LIBC/runtest.exe -w entry-static.exe inet_ntop_v4mapped"
    "/$LIBC/runtest.exe -w entry-static.exe inet_pton_empty_last_field"
    "/$LIBC/runtest.exe -w entry-static.exe iswspace_null"
    "/$LIBC/runtest.exe -w entry-static.exe lrand48_signextend"
    "/$LIBC/runtest.exe -w entry-static.exe malloc_0"
    "/$LIBC/runtest.exe -w entry-static.exe mbsrtowcs_overflow"
    "/$LIBC/runtest.exe -w entry-static.exe memmem_oob_read"
    "/$LIBC/runtest.exe -w entry-static.exe memmem_oob"
    "/$LIBC/runtest.exe -w entry-static.exe mkdtemp_failure"
    "/$LIBC/runtest.exe -w entry-static.exe mkstemp_failure"
    "/$LIBC/runtest.exe -w entry-static.exe printf_1e9_oob"
    "/$LIBC/runtest.exe -w entry-static.exe printf_fmt_g_round"
    "/$LIBC/runtest.exe -w entry-static.exe printf_fmt_g_zeros"
    "/$LIBC/runtest.exe -w entry-static.exe printf_fmt_n"
    "/$LIBC/runtest.exe -w entry-static.exe putenv_doublefree"
    "/$LIBC/runtest.exe -w entry-static.exe regex_backref_0"
    "/$LIBC/runtest.exe -w entry-static.exe regex_bracket_icase"
    "/$LIBC/runtest.exe -w entry-static.exe regex_negated_range"
    "/$LIBC/runtest.exe -w entry-static.exe regexec_nosub"
    "/$LIBC/runtest.exe -w entry-static.exe scanf_bytes_consumed"
    "/$LIBC/runtest.exe -w entry-static.exe scanf_match_literal_eof"
    "/$LIBC/runtest.exe -w entry-static.exe scanf_nullbyte_char"
    "/$LIBC/runtest.exe -w entry-static.exe sigprocmask_internal"
    "/$LIBC/runtest.exe -w entry-static.exe sscanf_eof"
    "/$LIBC/runtest.exe -w entry-static.exe strverscmp"
    "/$LIBC/runtest.exe -w entry-static.exe syscall_sign_extend"
    "/$LIBC/runtest.exe -w entry-static.exe uselocale_0"
    "/$LIBC/runtest.exe -w entry-static.exe wcsncpy_read_overflow"
    "/$LIBC/runtest.exe -w entry-static.exe wcsstr_false_negative"
)

testcases_type=(
    "basic"
    "busybox"
    "lua"
    "libctest"
)

IMG_URL=https://github.com/Azure-stars/testsuits-for-oskernel/releases/download/v0.1/sdcard-$ARCH.img.gz
if [ ! -f sdcard-$ARCH.img ]; then
    echo -e "${CYAN_C}Downloading${END_C} $IMG_URL"
    wget -q $IMG_URL
    gunzip sdcard-$ARCH.img.gz
    if [ $? -ne 0 ]; then
        echo -e "${RED_C}download failed!${END_C}"
        exit 1
    fi
fi

cp sdcard-$ARCH.img $AX_ROOT/disk.img

ARG="AX_TESTCASE=oscomp ARCH=$ARCH EXTRA_CONFIG=../configs/$ARCH.toml BLK=y NET=y FEATURES=fp_simd,lwext4_rs SMP=4 ACCEL=n LOG=off"

echo -e "${GREEN_C}ARGS:${END_C} $ARG"
if [ $? -ne 0 ]; then
    echo -e "${RED_C}build failed!${END_C}"
fi

function test_one() {
    local testcase_type=$1
    local actual="apps/oscomp/actual_$testcase_type.out"
    RUN_TIME=$( { time { timeout --foreground $TIMEOUT make -C "$ROOT" $ARG run > "$actual" ; }; } )
    local res=$?
    if [ $res == 124 ]; then
        res=$S_TIMEOUT
    elif [ $res -ne 0 ]; then
        res=$S_FAILED
    else 
        res=$S_PASS
    fi
    cat "$actual"
    if [ $res -ne $S_PASS ]; then
        EXIT_STATUS=$res
        if [ $res == $S_FAILED ]; then
            echo -e "${RED_C}failed!${END_C} $RUN_TIME"
        elif [ $res == $S_TIMEOUT ]; then
            echo -e "${YELLOW_C}timeout!${END_C} $RUN_TIME"
        elif [ $res == $S_BUILD_FAILED ]; then
            echo -e "${RED_C}build failed!${END_C}"
        fi
        echo -e "${RED_C}actual output${END_C}:"
    else
        local judge_script="${ROOT}apps/oscomp/judge_${testcase_type}.py"
        python3 $judge_script < "$actual"
        if [ $? -ne 0 ]; then
            echo -e "${RED_C}failed!${END_C}"
            EXIT_STATUS=$S_FAILED
        else
            echo -e "${GREEN_C}passed!${END_C} $RUN_TIME"
            rm -f "$actual"
        fi
    fi
}

for type in "${testcases_type[@]}"; do
    declare -n test_list="${type}_testlist"
    echo -e "${CYAN_C}Testing $type testcases${END_C}"

    # clean the testcase_list file
    rm -f $ROOT/apps/oscomp/testcase_list
    for t in "${test_list[@]}"; do
        echo $t >> $ROOT/apps/oscomp/testcase_list
    done
    test_one "$type"
done

echo -e "test script exited with: $EXIT_STATUS"
exit $EXIT_STATUS
