#include <stdio.h>
#include <stdlib.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/time.h>
#include <string.h>
#include <sys/stat.h>
#include <assert.h>

#define FILENAME "testfile.bin"

const int PAGE_SIZE = 4096;

// 获取当前时间（秒精度浮点数）
double get_time() {
    struct timeval tv;
    gettimeofday(&tv, NULL);
    return tv.tv_sec + tv.tv_usec * 1e-6;
}

void test(int use_page_cache, size_t file_size, int round) {
    double start, end;
    double total_size = file_size * round;

    // 确保缓冲区大小对齐
    // // 写入的内容
    // char *a = (char *)malloc((file_size + PAGE_SIZE - 1) / PAGE_SIZE * PAGE_SIZE);
    // // 读取的内容
    // char *b = (char *)malloc((file_size + PAGE_SIZE - 1) / PAGE_SIZE * PAGE_SIZE);
    char *a = malloc(file_size);
    char *b = malloc(file_size);
    
    for (int i = 0; i < file_size; i++) {
        a[i] = rand() % 15;
    }

    // 打开文件
    int flag = O_WRONLY|O_CREAT|O_TRUNC;
    if (use_page_cache == 0) {
        flag |= O_DIRECT;
    }
    int fd = open(FILENAME, flag, 0644);
    if (fd < 0) {
        perror("文件创建失败");
        exit(EXIT_FAILURE);
    }

    // 重复 n 轮写入数据
    start = get_time();
    for (int i = 0; i < round; i++) {
        lseek(fd, 0, SEEK_SET);
        size_t cur = 0;
        while (cur < file_size) {
            ssize_t add = write(fd, a + cur, file_size - cur); // 从 a+cur 写入
            if (add <= 0) { // 修正5：处理错误
                perror("写入失败");
                close(fd);
                exit(EXIT_FAILURE);
            }
            cur += add;
        }
    }
    end = get_time();
    printf("[写入] 大小: %.2f MB, 耗时: %.2f s, 速度: %.2f MB/s\n",
           total_size / (1024.0 * 1024), end - start, total_size / (end - start) / (1024 * 1024));
    
    // 关闭文件前获取文件大小，此时应该从 page cache 中读取
    struct stat file_stat;
    off_t stat_size;
    stat(FILENAME, &file_stat);
    stat_size = file_stat.st_size;
    if (stat_size != file_size && use_page_cache == 1) {
        printf("(close 前) 文件大小: %ld, 理应 %ld\n", stat_size, file_size);
        close(fd);
        exit(EXIT_FAILURE);
    }
    
    // 关闭文件
    close(fd);
    
    // 关闭文件后获取文件大小，此时应该从磁盘读取
    stat(FILENAME, &file_stat);
    stat_size = file_stat.st_size;
    if (stat_size != file_size) {
        printf("(close 后) 文件大小: %ld, 理应 %ld\n", stat_size, file_size);
        exit(EXIT_FAILURE);
    }

    // 重新打开文件
    flag = O_RDONLY;
    if (use_page_cache == 0) {
        flag |= O_DIRECT;
    }
    if ((fd = open(FILENAME, flag)) < 0) {
        perror("文件打开失败");
        exit(EXIT_FAILURE);
    }

    // 重复 n 轮读取数据
    start = get_time();
    for (int i = 0; i < round; i++) {
        lseek(fd, 0, SEEK_SET);
        ssize_t cur = 0;
        while (cur < file_size) {
            ssize_t add = read(fd, b + cur, file_size - cur); // 读到 b+cur
            if (add <= 0) { // 修正5：处理错误
                perror("读取失败");
                close(fd);
                exit(EXIT_FAILURE);
            }
            cur += add;
        }
    }
    close(fd);
    end = get_time();
    printf("[读取] 大小: %.2f MB, 耗时: %.2f s, 速度: %.2f MB/s\n",
        total_size / (1024.0 * 1024), end - start, total_size / (end - start) / (1024 * 1024));

    // 验证读写的内容一样
    for (int i = 0; i < file_size; i++) {
        // printf(" a[%d] = %d, b[%d] = %d\n", i, a[i], i, b[i]);
        if (a[i] != b[i]) {
            printf("读写内容不一致, a[%d] = %d, b[%d] = %d\n", i, a[i], i, b[i]);
            exit(EXIT_FAILURE);
        }
    }
    printf("读写数据一致\n");

    // 清理
    unlink(FILENAME);
    free(a);
    free(b);
}

int main() {
    printf("使用 page cache：\n");
    test(1, 4096 * 200, 1);
    
    // printf("关闭 page cache，直接 io：\n");
    // test(0, 513, 1);

    printf("SUCCESS PAGE CACHE TEST!\n");
    return 0;
}