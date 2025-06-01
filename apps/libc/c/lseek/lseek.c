#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <assert.h>

#define GAP (12345)

int main(int argc, char *argv[]) {
    const char *filename = "testfile";
    FILE *fp;

    // 创建空文件
    fp = fopen(filename, "wb");
    if (fp == NULL) {
        perror("Failed to create file");
        return 1;
    }

    // 获取初始文件大小
    if (fseek(fp, 0, SEEK_END) != 0) {
        perror("fseek failed");
        fclose(fp);
        return 1;
    }
    long initial_size = ftell(fp);

    // 在文件末尾后1KB处写入
    if (fseek(fp, initial_size + GAP, SEEK_SET) != 0) {
        perror("fseek failed");
        fclose(fp);
        return 1;
    }

    const char *test_data = "TEST DATA";
    if (fwrite(test_data, 1, strlen(test_data), fp) != strlen(test_data)) {
        perror("Failed to write test data");
        fclose(fp);
        return 1;
    }

    // 关闭文件后重新打开以获取准确大小
    fclose(fp);

    // 验证文件大小
    fp = fopen(filename, "rb");
    if (fp == NULL) {
        perror("Failed to reopen file");
        return 1;
    }

    if (fseek(fp, 0, SEEK_END) != 0) {
        perror("fseek failed");
        fclose(fp);
        return 1;
    }
    long final_size = ftell(fp);

    // 验证
    long expected_size = initial_size + GAP + strlen(test_data);
    if (final_size == expected_size) {
        printf("文件 size 验证通过\n");
    } else {
        printf("文件 size 验证失败: 期望大小为 %ld 字节，但实际为 %ld 字节\n", expected_size, final_size);
        fclose(fp);
        return 1;
    }

    fseek(fp, 0, SEEK_SET);

    char *buf = (char *)malloc(GAP);
    int len = fread(buf, 1, GAP, fp);
    for (int i = 0; i < len; i++) {
        assert(buf[i] == 0);
    }
    fclose(fp);

    printf("lseek test SUCCESS\n");
    return 0;
}
    