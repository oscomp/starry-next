#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#define FILE_SIZE 4096  // 4KB

int main() {
    const char *filename = "test_file.bin";
    char *write_buffer = NULL;
    char *read_buffer = NULL;
    FILE *file = NULL;
    int result = EXIT_FAILURE;
    time_t t;

    // 初始化随机数种子
    srand((unsigned) time(&t));

    // 分配内存
    write_buffer = (char *)malloc(FILE_SIZE);
    read_buffer = (char *)malloc(FILE_SIZE);
    if (write_buffer == NULL || read_buffer == NULL) {
        fprintf(stderr, "内存分配失败\n");
        goto cleanup;
    }

    // 生成随机数据
    for (int i = 0; i < FILE_SIZE; i++) {
        write_buffer[i] = (char)(rand() % 256);
    }

    // 写入文件
    file = fopen(filename, "wb");
    if (file == NULL) {
        fprintf(stderr, "无法创建文件\n");
        goto cleanup;
    }

    if (fwrite(write_buffer, 1, FILE_SIZE, file) != FILE_SIZE) {
        fprintf(stderr, "写入文件失败\n");
        fclose(file);
        goto cleanup;
    }
    fclose(file);
    file = NULL;

    // 读取文件
    file = fopen(filename, "rb");
    if (file == NULL) {
        fprintf(stderr, "无法打开文件\n");
        goto cleanup;
    }

    long length = fread(read_buffer, 1, FILE_SIZE, file);

    if (length != FILE_SIZE) {
        fprintf(stderr, "读取文件失败， 读取大小: %ld\n", length);
        fclose(file);
        goto cleanup;
    }
    fclose(file);
    file = NULL;

    // 验证内容
    if (memcmp(write_buffer, read_buffer, FILE_SIZE) == 0) {
        printf("验证成功：写入和读取的内容一致\n");
        result = EXIT_SUCCESS;
    } else {
        printf("验证失败：写入和读取的内容不一致\n");
    }

cleanup:
    // 释放资源
    if (file != NULL) {
        fclose(file);
    }
    if (write_buffer != NULL) {
        free(write_buffer);
    }
    if (read_buffer != NULL) {
        free(read_buffer);
    }

    return result;
}    