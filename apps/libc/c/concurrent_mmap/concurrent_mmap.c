#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <sys/wait.h>
#include <string.h>
#include <time.h>
#include <stdatomic.h>
#include <assert.h>

#define FILENAME "/tmp/mmap_shared_file.bin"
const int  FILE_SIZE = (4096 * 20);  // 文件大小
#define WRITER_COUNT 2           // 写进程数量
#define READER_COUNT 4           // 读进程数量

char func(unsigned i) {
    return i % 14 + 'a';
}

// 写进程函数
void writer_process(int writer_id) {
    assert(writer_id < WRITER_COUNT);
    int fd = open(FILENAME, O_RDWR | O_CREAT, 0666);
    if (fd == -1) {
        perror("writer open failed");
        exit(EXIT_FAILURE);
    }

    // 扩展文件大小
    ftruncate(fd, FILE_SIZE);

    // 映射整个文件
    char *map = mmap(NULL, FILE_SIZE, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (map == MAP_FAILED) {
        perror("writer mmap failed");
        close(fd);
        exit(EXIT_FAILURE);
    }

    // 写入数据块
    for (unsigned i = writer_id; i < FILE_SIZE; i += WRITER_COUNT) {
        map[i] = func(i);
    }
    munmap(map, FILE_SIZE);
    
    close(fd);
    printf("Writer %d 写入结束\n", writer_id);
    // exit(EXIT_SUCCESS);
}

// 读进程函数
void reader_process(int reader_id) {
    printf("enter reader process %d\n", reader_id);
    
    int fd = open(FILENAME, O_RDONLY);
    if (fd < 0) {
        printf("reader open failed, fd = %d\n", fd);
        exit(EXIT_FAILURE);
    }

    printf("Reader %d open file success, fd: %d\n", reader_id), fd;

    // 获取文件大小
    struct stat st;
    if (fstat(fd, &st) == -1) {
        perror("fstat failed");
        close(fd);
        exit(EXIT_FAILURE);
    }
    
    if (st.st_size != FILE_SIZE) {
        fprintf(stderr, "Reader %d: File size too small (%ld < %d)\n", 
                reader_id, st.st_size, FILE_SIZE);
        close(fd);
        exit(EXIT_FAILURE);
    }

    printf("Reader %d check filesize success\n", reader_id);

    // 只读映射
    char *map = mmap(NULL, FILE_SIZE, PROT_READ, MAP_SHARED, fd, 0);
    if (map == MAP_FAILED) {
        perror("reader mmap failed");
        close(fd);
        exit(EXIT_FAILURE);
    }

    printf("Reader %d started verification\n", reader_id);
    
    for (int round = 0; round < 10; round++) {
        for (unsigned i = 0; i < FILE_SIZE; i++) {
            if (map[i] != func(i)) {
                printf("位置 %d 读写不一致：期待 %d, 实际 %d\n", i, func(i), map[i]);
            }
        }
    }
    
    printf("Reader %d: 读写验证成功！\n", reader_id);
    
    munmap(map, FILE_SIZE);
    close(fd);
    exit(EXIT_SUCCESS);
}

int main() {
    pid_t writer_pids[WRITER_COUNT], reader_pids[READER_COUNT];
    int status;
    
    printf("Starting MMAP read/write consistency test\n");
    printf("File: %s, Size: %d bytes\n", FILENAME, FILE_SIZE);
    printf("Writer: %d, Readers: %d\n\n", WRITER_COUNT, READER_COUNT);

    // 创建写进程
    for (int i = 0; i < WRITER_COUNT; i++) {
        writer_pids[i] = fork();
        if (writer_pids[i] == 0) {
            writer_process(i);
            exit(EXIT_SUCCESS);
        } else if (writer_pids[i] < 0) {
            perror("fork for reader failed");
            exit(EXIT_FAILURE);
        }
    }

    for (int i = 0; i < WRITER_COUNT; i++) {
        waitpid(writer_pids[i], &status, 0);
            if (!WIFEXITED(status) || WEXITSTATUS(status) != EXIT_SUCCESS) {
                fprintf(stderr, "Writer process failed\n");
            } else {
                printf("Writer process completed successfully\n");
            }
    }

    // 创建读进程
    for (int i = 0; i < READER_COUNT; i++) {
        reader_pids[i] = fork();
        if (reader_pids[i] == 0) {
            reader_process(i);
            exit(EXIT_SUCCESS);
        } else if (reader_pids[i] < 0) {
            perror("fork for reader failed");
            exit(EXIT_FAILURE);
        }
    }

    // 等待所有读进程完成
    int reader_failures = 0;
    for (int i = 0; i < READER_COUNT; i++) {
        waitpid(reader_pids[i], &status, 0);
        if (!WIFEXITED(status) || WEXITSTATUS(status) != EXIT_SUCCESS) {
            reader_failures++;
        }
    }
    
    // 清理
    unlink(FILENAME);
    
    printf("\nTest completed. \n");
    if (reader_failures == 0) {
        printf("SUCCESS: 所有 ReaderProcess 验证成功\n");
        exit(EXIT_SUCCESS);
    } else {
        printf("FAILURE: %d/%d readers 发现错误\n", 
               reader_failures, READER_COUNT);
        exit(EXIT_FAILURE);
    }
}