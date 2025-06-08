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
#define WRITER_COUNT 1           // 写进程数量
#define READER_COUNT 4           // 读进程数量

char func(unsigned i) {
    return i % 14;
}

// 写进程函数
void writer_process(int writer_id) {
    // int fd = open(FILENAME, O_RDWR | O_CREAT | O_TRUNC, 0666);
    // if (fd == -1) {
    //     perror("writer open failed");
    //     exit(EXIT_FAILURE);
    // }

    // // 扩展文件大小
    // if (lseek(fd, FILE_SIZE - 1, SEEK_SET) == -1) {
    //     perror("lseek failed");
    //     close(fd);
    //     exit(EXIT_FAILURE);
    // }
    // write(fd, "", 1);
    // lseek(fd, 0, SEEK_SET);
    // fsync(fd);

    // // 映射整个文件
    // char *map = mmap(NULL, FILE_SIZE, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    // if (map == MAP_FAILED) {
    //     perror("writer mmap failed");
    //     close(fd);
    //     exit(EXIT_FAILURE);
    // }

    // // 写入数据块
    // for (unsigned i = 0; i < FILE_SIZE; i++) {
    //     map[i] = func(i);
    // }

    // for (unsigned i = 0; i < FILE_SIZE; i++) {
    //     assert(map[i] == func(i));
    // }
    // munmap(map, FILE_SIZE);
    // close(fd);
}

// 读进程函数
void reader_process(int reader_id) {
    // int fd = open(FILENAME, O_RDONLY);
    // if (fd < 0) {
    //     perror("reader open failed");
    //     exit(EXIT_FAILURE);
    // }
    
    // char tmp[10];
    // printf("before read\n");
    // lseek(fd, 0, SEEK_SET);

    // // 蜜汁爆栈？
    // int n = read(fd, &tmp, 5);
    // printf("read n %d\n", n);
    // printf("read %d; fd tmp: %d %d %d !\n", n, tmp[0], tmp[1], tmp[2]);

    // // 获取文件大小
    // struct stat st;
    // if (fstat(fd, &st) == -1) {
    //     perror("fstat failed");
    //     close(fd);
    //     exit(EXIT_FAILURE);
    // }

    // if (st.st_size < FILE_SIZE) {
    //     fprintf(stderr, "Reader %d: File size too small (%ld < %d)\n", 
    //             reader_id, st.st_size, FILE_SIZE);
    //     close(fd);
    //     exit(EXIT_FAILURE);
    // }
    // printf("after read\n");

    // // 只读映射
    // char *map = mmap(NULL, FILE_SIZE, PROT_READ, 1, fd, 0);
    // if (map == MAP_FAILED) {
    //     perror("reader mmap failed");
    //     close(fd);
    //     exit(EXIT_FAILURE);
    // }

    // printf("Reader %d started verification\n", reader_id);
    
    for (unsigned i = 0; i < FILE_SIZE; i++) {
        // printf("read pos %d\n", i);
        // char x = map[i];
        // if (map[i] != func(i)) {
        //     // printf("读写不一致：期待 %d, 实际 %d\n", func(i), map[i]);
        //     printf("i = %d\n", i);
        //     exit(EXIT_FAILURE);
        // }
    }
    
    printf("Reader %d: Success\n", reader_id);
    
    // munmap(map, FILE_SIZE);
    // close(fd);
    exit(EXIT_SUCCESS);
}

int main() {
    pid_t writer_pid, reader_pids[READER_COUNT];
    int status;
    
    printf("Starting MMAP read/write consistency test\n");
    printf("File: %s, Size: %d bytes\n", FILENAME, FILE_SIZE);
    // printf("Writer: %d, Readers: %d\n\n", WRITER_COUNT, READER_COUNT);

    // writer_pid = fork();
    if (writer_pid == 0) {
        writer_process(0);
        exit(EXIT_SUCCESS);
    } else if (writer_pid < 0) {
        perror("fork for writer failed");
        exit(EXIT_FAILURE);
    } else {
        waitpid(writer_pid, &status, 0);
    }
    writer_process(0);
    
    printf("finish write\n");

    // reader_process(0);

    pid_t reader_pid = fork();
    if (reader_pids == 0) {
        reader_process(0);
        exit(EXIT_SUCCESS);
    } else if (reader_pid < 0) {
        perror("fork for writer failed");
        exit(EXIT_FAILURE);
    } else {
        waitpid(reader_pid, &status, 0);
    }

}