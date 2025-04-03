import json
import sys

# TODO: Add more commands to test here
libctest_baseline = """
========== START entry-static.exe argv ==========
Pass!
========== END entry-static.exe argv ==========
========== START entry-static.exe basename ==========
Pass!
========== END entry-static.exe basename ==========
========== START entry-static.exe dirname ==========
Pass!
========== END entry-static.exe dirname ==========
========== START entry-static.exe env ==========
Pass!
========== END entry-static.exe env ==========
========== START entry-static.exe fdopen ==========
Pass!
========== END entry-static.exe fdopen ==========
========== START entry-static.exe inet_pton ==========
Pass!
========== END entry-static.exe inet_pton ==========
========== START entry-static.exe memstream ==========
Pass!
========== END entry-static.exe memstream ==========
========== START entry-static.exe random ==========
Pass!
========== END entry-static.exe random ==========
========== START entry-static.exe search_hsearch ==========
Pass!
========== END entry-static.exe search_hsearch ==========
========== START entry-static.exe search_insque ==========
Pass!
========== END entry-static.exe search_insque ==========
========== START entry-static.exe search_lsearch ==========
Pass!
========== END entry-static.exe search_lsearch ==========
========== START entry-static.exe search_tsearch ==========
Pass!
========== END entry-static.exe search_tsearch ==========
========== START entry-static.exe snprintf ==========
Pass!
========== END entry-static.exe snprintf ==========
========== START entry-static.exe string ==========
Pass!
========== END entry-static.exe string ==========
========== START entry-static.exe string_memcpy ==========
Pass!
========== END entry-static.exe string_memcpy ==========
========== START entry-static.exe string_memmem ==========
Pass!
========== END entry-static.exe string_memmem ==========
========== START entry-static.exe string_memset ==========
Pass!
========== END entry-static.exe string_memset ==========
========== START entry-static.exe string_strchr ==========
Pass!
========== END entry-static.exe string_strchr ==========
========== START entry-static.exe string_strcspn ==========
Pass!
========== END entry-static.exe string_strcspn ==========
========== START entry-static.exe string_strstr ==========
Pass!
========== END entry-static.exe string_strstr ==========
========== START entry-static.exe strptime ==========
Pass!
========== END entry-static.exe strptime ==========
========== START entry-static.exe strtod ==========
Pass!
========== END entry-static.exe strtod ==========
========== START entry-static.exe strtod_simple ==========
Pass!
========== END entry-static.exe strtod_simple ==========
========== START entry-static.exe strtof ==========
Pass!
========== END entry-static.exe strtof ==========
========== START entry-static.exe strtold ==========
Pass!
========== END entry-static.exe strtold ==========
========== START entry-static.exe tgmath ==========
Pass!
========== END entry-static.exe tgmath ==========
========== START entry-static.exe time ==========
Pass!
========== END entry-static.exe time ==========
========== START entry-static.exe tls_align ==========
Pass!
========== END entry-static.exe tls_align ==========
Pass!
========== START entry-static.exe udiv ==========
Pass!
========== END entry-static.exe udiv ==========
========== START entry-static.exe wcsstr ==========
Pass!
========== END entry-static.exe wcsstr ==========
========== START entry-static.exe fgets_eof ==========
Pass!
========== END entry-static.exe fgets_eof ==========
========== START entry-static.exe inet_ntop_v4mapped ==========
Pass!
========== END entry-static.exe inet_ntop_v4mapped ==========
========== START entry-static.exe inet_pton_empty_last_field ==========
Pass!
========== END entry-static.exe inet_pton_empty_last_field ==========
========== START entry-static.exe iswspace_null ==========
Pass!
========== END entry-static.exe iswspace_null ==========
========== START entry-static.exe lrand48_signextend ==========
Pass!
========== END entry-static.exe lrand48_signextend ==========
========== START entry-static.exe malloc_0 ==========
Pass!
========== END entry-static.exe malloc_0 ==========
========== START entry-static.exe mbsrtowcs_overflow ==========
Pass!
========== END entry-static.exe mbsrtowcs_overflow ==========
========== START entry-static.exe memmem_oob_read ==========
Pass!
========== END entry-static.exe memmem_oob_read ==========
========== START entry-static.exe memmem_oob ==========
Pass!
========== END entry-static.exe memmem_oob ==========
========== START entry-static.exe mkdtemp_failure ==========
Pass!
========== END entry-static.exe mkdtemp_failure ==========
========== START entry-static.exe mkstemp_failure ==========
Pass!
========== END entry-static.exe mkstemp_failure ==========
========== START entry-static.exe printf_1e9_oob ==========
Pass!
========== END entry-static.exe printf_1e9_oob ==========
========== START entry-static.exe printf_fmt_g_round ==========
Pass!
========== END entry-static.exe printf_fmt_g_round ==========
========== START entry-static.exe printf_fmt_g_zeros ==========
Pass!
========== END entry-static.exe printf_fmt_g_zeros ==========
========== START entry-static.exe printf_fmt_n ==========
Pass!
========== END entry-static.exe printf_fmt_n ==========
========== START entry-static.exe putenv_doublefree ==========
Pass!
========== END entry-static.exe putenv_doublefree ==========
========== START entry-static.exe regex_backref_0 ==========
Pass!
========== END entry-static.exe regex_backref_0 ==========
========== START entry-static.exe regex_bracket_icase ==========
Pass!
========== END entry-static.exe regex_bracket_icase ==========
========== START entry-static.exe regex_negated_range ==========
Pass!
========== END entry-static.exe regex_negated_range ==========
========== START entry-static.exe regexec_nosub ==========
Pass!
========== END entry-static.exe regexec_nosub ==========
========== START entry-static.exe scanf_bytes_consumed ==========
Pass!
========== END entry-static.exe scanf_bytes_consumed ==========
========== START entry-static.exe scanf_match_literal_eof ==========
Pass!
========== END entry-static.exe scanf_match_literal_eof ==========
========== START entry-static.exe scanf_nullbyte_char ==========
Pass!
========== END entry-static.exe scanf_nullbyte_char ==========
========== START entry-static.exe sigprocmask_internal ==========
Pass!
========== END entry-static.exe sigprocmask_internal ==========
========== START entry-static.exe sscanf_eof ==========
Pass!
========== END entry-static.exe sscanf_eof ==========
========== START entry-static.exe strverscmp ==========
Pass!
========== END entry-static.exe strverscmp ==========
========== START entry-static.exe syscall_sign_extend ==========
Pass!
========== END entry-static.exe syscall_sign_extend ==========
========== START entry-static.exe uselocale_0 ==========
Pass!
========== END entry-static.exe uselocale_0 ==========
========== START entry-static.exe wcsncpy_read_overflow ==========
Pass!
========== END entry-static.exe wcsncpy_read_overflow ==========
========== START entry-static.exe wcsstr_false_negative ==========
Pass!
========== END entry-static.exe wcsstr_false_negative ==========
"""

def parse_libctest(output):
    ans = {}
    key = ""
    for line in output.split("\n"):
        line = line.replace('\n', '').replace('\r', '')
        if "START entry-static.exe" in line:
            key = "libctest static " + line.split(" ")[3]
        elif "START entry-dynamic.exe" in line:
            key = "libctest dynamic " + line.split(" ")[3]
        if line == "Pass!" and key != "":
            ans[key] = 1
    return ans

serial_out = sys.stdin.read()
libctest_baseline_out = parse_libctest(libctest_baseline)
libctest_output = parse_libctest(serial_out)
for k in libctest_baseline_out.keys():
    if k not in libctest_output:
        libctest_output[k] = 0

results = [{
    "name": k,
    "pass": v,
    "total": 1,
    "score": v,
} for k, v in libctest_output.items()]
for r in results:
    if r["score"] == 0:
        print(f"libctest testcase {r['name']} failed")
        exit(255)

print("libctest testcases passed")
print(json.dumps(results))