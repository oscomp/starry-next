#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/stat.h>
#include <string.h>
#include <errno.h>

#define TEST_FILE "ftruncate_test.txt"
#define BUFFER_SIZE 1024

int main() {
    int fd;
    struct stat file_stat;
    char buffer[BUFFER_SIZE] = "Hello, this is a test file for ftruncate.\n";
    char read_buffer[BUFFER_SIZE] = {0};

    // 创建测试文件并写入数据
    fd = open(TEST_FILE, O_CREAT | O_RDWR, S_IRUSR | S_IWUSR);
    if (fd == -1) {
        perror("open failed");
        return EXIT_FAILURE;
    }

    // 写入初始数据
    if (write(fd, buffer, strlen(buffer)) == -1) {
        perror("write failed");
        close(fd);
        return EXIT_FAILURE;
    }

    // 测试场景1: 扩展文件大小
    printf("=== 测试场景1: 扩展文件大小 ===\n");
    if (ftruncate(fd, 2048) == -1) {
        perror("ftruncate (expand) failed");
        close(fd);
        return EXIT_FAILURE;
    }

    // 验证文件大小
    if (fstat(fd, &file_stat) == -1) {
        perror("fstat failed");
        close(fd);
        return EXIT_FAILURE;
    }
    printf("扩展后文件大小: %ld 字节\n", (long)file_stat.st_size);

    // 测试场景2: 读取扩展后文件
    printf("=== 测试场景2: 读取扩展后文件 ===\n");
    lseek(fd, 0, SEEK_SET);
    ssize_t bytes_read = read(fd, read_buffer, BUFFER_SIZE);
    if (bytes_read == -1) {
        perror("read failed");
        close(fd);
        return EXIT_FAILURE;
    }
    printf("读取内容: %.*s\n", (int)bytes_read, read_buffer);

    // 测试场景3: 截断文件
    printf("=== 测试场景3: 截断文件 ===\n");
    if (ftruncate(fd, 10) == -1) {
        perror("ftruncate (truncate) failed");
        close(fd);
        return EXIT_FAILURE;
    }

    // 验证截断后文件大小
    if (fstat(fd, &file_stat) == -1) {
        perror("fstat failed");
        close(fd);
        return EXIT_FAILURE;
    }
    printf("截断后文件大小: %ld 字节\n", (long)file_stat.st_size);

    // 测试场景4: 读取截断后文件
    lseek(fd, 0, SEEK_SET);
    memset(read_buffer, 0, BUFFER_SIZE);
    bytes_read = read(fd, read_buffer, BUFFER_SIZE);
    if (bytes_read == -1) {
        perror("read failed");
        close(fd);
        return EXIT_FAILURE;
    }
    printf("截断后读取内容: %.*s\n", (int)bytes_read, read_buffer);

    // 测试场景5: 截断到0
    printf("=== 测试场景5: 截断到0 ===\n");
    if (ftruncate(fd, 0) == -1) {
        perror("ftruncate (zero) failed");
        close(fd);
        return EXIT_FAILURE;
    }

    if (fstat(fd, &file_stat) == -1) {
        perror("fstat failed");
        close(fd);
        return EXIT_FAILURE;
    }
    printf("截断到0后文件大小: %ld 字节\n", (long)file_stat.st_size);

    // 测试场景6: 写入截断为0的文件
    printf("=== 测试场景6: 写入截断为0的文件 ===\n");
    const char *new_data = "New data after truncate.\n";
    if (write(fd, new_data, strlen(new_data)) == -1) {
        perror("write after truncate failed");
        close(fd);
        return EXIT_FAILURE;
    }

    lseek(fd, 0, SEEK_SET);
    memset(read_buffer, 0, BUFFER_SIZE);
    bytes_read = read(fd, read_buffer, BUFFER_SIZE);
    if (bytes_read == -1) {
        perror("read after write failed");
        close(fd);
        return EXIT_FAILURE;
    }
    printf("写入后读取内容: %.*s\n", (int)bytes_read, read_buffer);

    // 清理资源
    close(fd);
    if (unlink(TEST_FILE) == -1) {
        perror("unlink failed");
        return EXIT_FAILURE;
    }

    printf("=== 所有测试完成 ===\n");
    return EXIT_SUCCESS;
}    