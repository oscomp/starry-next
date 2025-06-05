#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/wait.h>
#include <sys/stat.h>
#include <time.h>
#include <errno.h>
#include <stdatomic.h>

#define FILE_NAME "test_file.dat"
#define BUFFER_SIZE 4096           // 缓冲区大小，通常为一个页
#define OPERATION_COUNT 100      // 每个进程的操作次数
#define CHILD_PROCESS_COUNT 8      // 子进程数量
#define FILE_SIZE (BUFFER_SIZE * OPERATION_COUNT * CHILD_PROCESS_COUNT) // 文件总大小

// 原子计数器，用于生成唯一的块编号
static atomic_int block_counter = 0;

// 数据块结构，包含序号和校验数据
typedef struct {
    int block_id;
    char data[BUFFER_SIZE - sizeof(int)];
} DataBlock;

// 生成测试数据
void generate_test_data(DataBlock *block, int block_id) {
    block->block_id = block_id;
    for (int i = 0; i < sizeof(block->data); i++) {
        block->data[i] = (char)(block_id ^ (i % 256));
    }
}

// 验证测试数据
int verify_test_data(const DataBlock *block) {
    for (int i = 0; i < sizeof(block->data); i++) {
        if (block->data[i] != (char)(block->block_id ^ (i % 256))) {
            printf("数据验证失败: 块 %d, 偏移 %d, 预期 %02X, 实际 %02X\n",
                   block->block_id, i, 
                   (unsigned char)(block->block_id ^ (i % 256)),
                   (unsigned char)block->data[i]);
            return 0;
        }
    }
    return 1;
}

// 预分配文件空间（替代fallocate）
void preallocate_file_space(int fd) {
    printf("预分配文件空间！");
    char zero = 0;
    off_t offset = FILE_SIZE - 1;
    
    // 将文件指针移动到末尾前一个字节
    if (lseek(fd, offset, SEEK_SET) == -1) {
        perror("lseek失败");
        exit(EXIT_FAILURE);
    }
    
    // 写入一个零字节
    if (write(fd, &zero, 1) != 1) {
        perror("写入失败");
        exit(EXIT_FAILURE);
    }
    
    // 确保文件系统更新
    if (fsync(fd) == -1) {
        perror("fsync失败");
        exit(EXIT_FAILURE);
    }
}

// 写入测试
void write_test(int fd, int process_id) {
    printf("开始并发分块写入");
    DataBlock block;
    off_t offset;
    ssize_t written;

    for (int i = 0; i < OPERATION_COUNT; i++) {
        int block_id = __atomic_fetch_add(&block_counter, 1, __ATOMIC_SEQ_CST);
        generate_test_data(&block, block_id);
        offset = (off_t)block_id * BUFFER_SIZE;

        written = pwrite(fd, &block, sizeof(block), offset);
        if (written != sizeof(block)) {
            perror("写入失败");
            exit(EXIT_FAILURE);
        }
    }
}

// 读取测试
void read_test(int fd, int process_id) {
    printf("开始并发分块读取");
    DataBlock block;
    off_t offset;
    ssize_t read_bytes;
    int verified = 1;

    for (int i = 0; i < OPERATION_COUNT; i++) {
        int block_id = process_id * OPERATION_COUNT + i;
        offset = (off_t)block_id * BUFFER_SIZE;

        read_bytes = pread(fd, &block, sizeof(block), offset);
        if (read_bytes != sizeof(block)) {
            perror("读取失败");
            exit(EXIT_FAILURE);
        }

        if (!verify_test_data(&block)) {
            verified = 0;
        }
    }

    if (!verified) {
        printf("进程 %d: 数据验证失败\n", process_id);
        exit(EXIT_FAILURE);
    }
}

int run_test(const char *test_type) {
    // 创建或打开文件
    int fd = open(FILE_NAME, 
                  test_type[0] == 'w' ? (O_CREAT | O_RDWR | O_TRUNC) : O_RDWR, 
                  0666);
    if (fd == -1) {
        perror("无法打开文件");
        return EXIT_FAILURE;
    }

    // 预分配文件空间（只在写入测试时需要）
    if (test_type[0] == 'w') {
        preallocate_file_space(fd);
    }

    clock_t start_time = clock();
    pid_t pid;
    int status;

    // 重置块计数器（用于写入测试）
    if (test_type[0] == 'w') {
        atomic_store(&block_counter, 0);
    }

    // 创建子进程
    for (int i = 0; i < CHILD_PROCESS_COUNT; i++) {
        pid = fork();
        if (pid < 0) {
            perror("创建子进程失败");
            close(fd);
            return EXIT_FAILURE;
        } else if (pid == 0) {
            // 子进程
            close(fd); // 关闭继承的文件描述符，重新打开以获得独立的文件偏移量
            fd = open(FILE_NAME, O_RDWR);
            if (fd == -1) {
                perror("子进程无法打开文件");
                exit(EXIT_FAILURE);
            }

            if (test_type[0] == 'w') {
                write_test(fd, i);
            } else if (test_type[0] == 'r') {
                read_test(fd, i);
            } else {
                printf("未知测试类型\n");
                close(fd);
                exit(EXIT_FAILURE);
            }

            close(fd);
            exit(EXIT_SUCCESS);
        }
    }

    // 父进程等待所有子进程完成
    for (int i = 0; i < CHILD_PROCESS_COUNT; i++) {
        wait(&status);
        if (!WIFEXITED(status) || WEXITSTATUS(status) != EXIT_SUCCESS) {
            printf("子进程异常退出\n");
            close(fd);
            return EXIT_FAILURE;
        }
    }

    clock_t end_time = clock();
    double elapsed_time = (double)(end_time - start_time) / CLOCKS_PER_SEC;

    // 计算性能指标
    size_t total_bytes = BUFFER_SIZE * OPERATION_COUNT * CHILD_PROCESS_COUNT;
    double throughput = (double)total_bytes / (1024 * 1024) / elapsed_time;

    printf("%s测试完成\n", test_type[0] == 'w' ? "写入" : "读取");
    printf("总操作时间: %.2f 秒\n", elapsed_time);
    printf("总数据量: %.2f MB\n", (double)total_bytes / (1024 * 1024));
    printf("吞吐量: %.2f MB/秒\n", throughput);
    printf("------------------------\n");

    close(fd);
    return EXIT_SUCCESS;
}

int main() {
    printf("开始文件并发读写测试...\n");
    printf("------------------------\n");

    // 执行写入测试
    printf("执行写入测试...\n");
    if (run_test("write") != EXIT_SUCCESS) {
        printf("写入测试失败\n");
        return EXIT_FAILURE;
    }

    // 执行读取测试
    printf("执行读取测试...\n");
    if (run_test("read") != EXIT_SUCCESS) {
        printf("读取测试失败\n");
        return EXIT_FAILURE;
    }

    printf("所有测试完成!\n");
    return EXIT_SUCCESS;
}    