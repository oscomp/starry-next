#include <stdio.h>
#include <stdlib.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/time.h>
#include <string.h>
#include <sys/stat.h>
#include <assert.h>

#define FILE_SIZE (1UL << 15)
#define BUFFER_SIZE (1 << 13)
#define ROUND 1000             // 测试轮次
#define TEST_SIZE  (FILE_SIZE * ROUND)
#define FILENAME "testfile.bin"

// 获取当前时间（秒精度浮点数）
double get_time() {
    struct timeval tv;
    gettimeofday(&tv, NULL);
    return tv.tv_sec + tv.tv_usec * 1e-6;
}

void test(int use_page_cache) {
    int fd;
    char *buffer;
    ssize_t ret;
    double start, end;
    size_t total = 0;

    // 分配对齐的内存缓冲区（提升性能）
    buffer = (char *)malloc(BUFFER_SIZE);
    memset(buffer, 0xAA, BUFFER_SIZE);  // 填充测试数据

    // ================== 写入测试 ==================
    int flag = O_WRONLY|O_CREAT|O_TRUNC;

    if (use_page_cache == 0) {
        flag |= O_DIRECT;
    }
    if ((fd = open(FILENAME, flag, 0644)) < 0) {
        perror("文件创建失败");
        exit(EXIT_FAILURE);
    }

    start = get_time();

    for (int i = 0; i < ROUND; i++) {
        // printf("round %d\n", i);
        for (total = 0; total < FILE_SIZE; total += ret) {
            size_t remaining = FILE_SIZE - total;
            size_t chunk = (remaining > BUFFER_SIZE) ? BUFFER_SIZE : remaining;
            
            ret = write(fd, buffer, chunk);
            if (ret < 0) {
                perror("写入失败");
                close(fd);
                exit(EXIT_FAILURE);
            }
        }
        assert(total == FILE_SIZE);
        lseek(fd, 0, SEEK_SET);
    }
    
    fsync(fd);  // 确保数据落盘
    end = get_time();
    
    // ================== 关闭文件前后大小测试 ==================
    struct stat file_stat;
    
    // 关闭文件前获取文件大小，此时应该从 page cache 中读取
    stat(FILENAME, &file_stat);
    off_t file_size = file_stat.st_size;
    // printf("文件大小: %ld, 理应 %ld\n", file_size, FILE_SIZE);
    assert(use_page_cache == 0 || file_size == FILE_SIZE);
    
    close(fd);
    
    // 关闭文件后获取文件大小，此时应该从磁盘读取
    stat(FILENAME, &file_stat);
    file_size = file_stat.st_size;
    // printf("文件大小: %ld, 理应 %ld\n", file_size, FILE_SIZE);
    assert(file_size == FILE_SIZE);

    printf("[写入] 大小: %.2f MB, 耗时: %.2f s, 速度: %.2f MB/s\n",
           TEST_SIZE / (1024.0 * 1024),
           end - start,
           TEST_SIZE / (end - start) / (1024 * 1024));
    // ================== 读取测试 ==================
    
    flag = O_RDONLY;
    if (use_page_cache == 0) {
        flag |= O_DIRECT;
    }
    if ((fd = open(FILENAME, flag)) < 0) {
        perror("文件打开失败");
        exit(EXIT_FAILURE);
    }
    start = get_time();
    
    for (int i = 0; i < ROUND; i++) {
        while ((ret = read(fd, buffer, BUFFER_SIZE))) {
            if (ret < 0) {
                perror("读取失败");
                close(fd);
                exit(EXIT_FAILURE);
            }
            total += ret;
        }
        lseek(fd, 0, SEEK_SET);
    }
    close(fd);
    end = get_time();

    printf("[读取] 大小: %.2f MB, 耗时: %.2f s, 速度: %.2f MB/s\n",
           TEST_SIZE / (1024.0 * 1024),
           end - start,
           TEST_SIZE / (end - start) / (1024 * 1024));

    // 清理
    unlink(FILENAME);
    free(buffer);
}

int main() {
    printf("使用 page cache：\n");
    test(1);

    printf("关闭 page cache，直接 io：\n");
    test(0);
    return 0;
}