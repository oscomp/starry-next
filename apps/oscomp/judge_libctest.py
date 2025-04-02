import json
import sys

# TODO: Add more commands to test here
libctest_baseline = """
========== START entry-static.exe argv ==========
Pass!
========== END entry-static.exe argv ==========
========== START entry-static.exe qsort ==========
Pass!
========== END entry-static.exe qsort ==========
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
        if (line == "Pass!") and key != "":
            ans[key] = 1
    return ans

serial_out = sys.stdin.read()
libctest_baseline_out = parse_libctest(libctest_baseline)
print("libctest_baseline_out:")
print(libctest_baseline_out)
libctest_output = parse_libctest(serial_out)
print("libctest_output:")
print(libctest_output)
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