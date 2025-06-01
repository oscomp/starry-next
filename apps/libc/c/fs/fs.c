#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <time.h>

#define FILENAME "testfile.bin"
#define MIN_SIZE 1024    // 1KB
#define MAX_SIZE 1048576 // 1MB

// 生成随机大小的内容
void generate_random_data(char *buffer, size_t size) {
    for (size_t i = 0; i < size; i++) {
        buffer[i] = rand() & 0xFF; // 生成0-255的随机字节
    }
}

int main() {
    srand(time(NULL)); // 初始化随机种子
    size_t file_size = MIN_SIZE + (rand() % (MAX_SIZE - MIN_SIZE + 1));
    char *write_buffer = malloc(file_size);
    char *read_buffer = malloc(file_size);
    
    if (!write_buffer || !read_buffer) {
        perror("内存分配失败");
        exit(EXIT_FAILURE);
    }

    // 生成随机数据
    generate_random_data(write_buffer, file_size);

    // 阶段1: 创建并写入文件
    FILE *fp = fopen(FILENAME, "wb");
    if (!fp) {
        perror("文件创建失败");
        exit(EXIT_FAILURE);
    }

    size_t written = fwrite(write_buffer, 1, file_size, fp);
    fclose(fp);

    if (written != file_size) {
        fprintf(stderr, "写入错误: 预期 %zu 字节, 实际 %zu 字节\n", 
                file_size, written);
        exit(EXIT_FAILURE);
    }

    // 阶段2: 验证文件大小
    struct stat st;
    if (stat(FILENAME, &st) != 0) {
        perror("文件状态获取失败");
        exit(EXIT_FAILURE);
    }

    if ((size_t)st.st_size != file_size) {
        fprintf(stderr, "大小不一致: 预期 %zu 字节, 实际 %ld 字节\n", 
                file_size, st.st_size);
        exit(EXIT_FAILURE);
    }

    // 阶段3: 读取并验证内容
    fp = fopen(FILENAME, "rb");
    if (!fp) {
        perror("文件打开失败");
        exit(EXIT_FAILURE);
    }

    size_t read = fread(read_buffer, 1, file_size, fp);
    fclose(fp);

    if (read != file_size) {
        fprintf(stderr, "读取错误: 预期 %zu 字节, 实际 %zu 字节\n", 
                file_size, read);
        exit(EXIT_FAILURE);
    }

    // 验证内容一致性
    if (memcmp(write_buffer, read_buffer, file_size)) {
        fprintf(stderr, "内容验证失败: 写入和读取内容不一致！\n");
        exit(EXIT_FAILURE);
    }

    // 清理
    free(write_buffer);
    free(read_buffer);
    remove(FILENAME); // 删除测试文件

    printf("测试成功！\n");
    printf("文件大小: %zu KB (%.2f MB)\n", 
           file_size / 1024, (float)file_size / (1024 * 1024));
    return EXIT_SUCCESS;
}