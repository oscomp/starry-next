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

#define FILENAME "mmap_shared_file.bin"
const int  FILE_SIZE = (4096 * 2);  // 文件大小

char func(unsigned i) {
    return i % 14;
}

// 写进程函数
void writer_process(int writer_id) {
    int fd = open(FILENAME, O_RDWR | O_CREAT | O_TRUNC, 0666);
    if (fd == -1) {
        perror("writer open failed");
        exit(EXIT_FAILURE);
    }

    // 扩展文件大小
    if (lseek(fd, FILE_SIZE - 1, SEEK_SET) == -1) {
        perror("lseek failed");
        close(fd);
        exit(EXIT_FAILURE);
    }
    write(fd, "", 1);
    lseek(fd, 0, SEEK_SET);
    fsync(fd);

    // 映射整个文件
    char *map = (char *)mmap(NULL, FILE_SIZE, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (map == MAP_FAILED) {
        perror("writer mmap failed");
        close(fd);
        exit(EXIT_FAILURE);
    }

    // 写入数据块
    for (unsigned i = 0; i < FILE_SIZE; i++) {
        map[i] = func(i);
    }
    if (munmap(map, FILE_SIZE) < 0) {
        perror("munmap failed");
    }
    close(fd);
}

// 读进程函数
void reader_process(int reader_id) {    
    int fd = open(FILENAME, O_RDONLY);
    if (fd < 0) {
        perror("reader open failed");
        exit(EXIT_FAILURE);
    }

    // 获取文件大小
    struct stat st;
    if (fstat(fd, &st) == -1 || st.st_size < FILE_SIZE) {
        perror("fstat 获取失败或文件大小错误");
        close(fd);
        exit(EXIT_FAILURE);
    }

    // 只读映射
    char *map = mmap(NULL, FILE_SIZE, PROT_READ, 1, fd, 0);
    printf("mmap addr: %p\n", map);
    
    if (map == 0 || map == MAP_FAILED) {
        perror("reader mmap failed");
        close(fd);
        exit(EXIT_FAILURE);
    }
    
    for (unsigned i = 0; i < FILE_SIZE; i++) {
        if (map[i] != func(i)) {
            printf("位置 %d 读写不一致, 期望 %d, 实际 %d\n", i, func(i), map[i]);
            exit(EXIT_FAILURE);
        }
    }
    
    printf("Reader %d: 验证成功，读写完全一致！\n", reader_id);
    
    munmap(map, FILE_SIZE);
    close(fd);
    exit(EXIT_SUCCESS);
}

int main() {
    pid_t writer_pid, reader_pid;
    
    writer_process(0);
    
    // reader_process(0);
    
    reader_pid = fork();
    if (reader_pid == 0) {
        reader_process(0);
        exit(EXIT_SUCCESS);
    } else if (reader_pid < 0) {
        perror("fork for reader failed");
        exit(EXIT_FAILURE);
    } else {
        int status;
        waitpid(reader_pid, &status, 0);
    }
}