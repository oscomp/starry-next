#include <stdio.h>
#include <stdlib.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/mman.h>
#include <string.h>

#define FILE_SIZE 4096  // 目标文件大小
#define TEST_STR "Hello, mmap world!"

int main() {
    int fd;
    char *mapped;
    const char *tmpfile = "mmap_test.tmp";

    // 1. 创建并打开文件
    fd = open(tmpfile, O_RDWR | O_CREAT | O_TRUNC, 0644);
    if (fd == -1) {
        perror("open failed");
        exit(EXIT_FAILURE);
    }

    // 2. 写入初始数据
    ssize_t written = write(fd, TEST_STR, strlen(TEST_STR));
    if (written == -1) {
        perror("write failed");
        close(fd);
        exit(EXIT_FAILURE);
    }

    // 3. 用 write 扩展文件到 FILE_SIZE
    // 方法：定位到 FILE_SIZE-1 并写入一个空字节
    if (lseek(fd, FILE_SIZE - 1, SEEK_SET) == -1) {
        perror("lseek failed");
        close(fd);
        exit(EXIT_FAILURE);
    }

    if (write(fd, "", 1) == -1) { // 写入 1 字节空字符
        perror("write failed");
        close(fd);
        exit(EXIT_FAILURE);
    }

    // 4. 映射文件到内存
    mapped = mmap(NULL, FILE_SIZE, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (mapped == MAP_FAILED) {
        perror("mmap failed");
        close(fd);
        exit(EXIT_FAILURE);
    }

    // 5. 测试读取文件内容
    printf("原始文件内容: %s\n", mapped);
    
    // 6. 测试修改文件内容
    const char *new_str = "This is modified via mmap!";
    memcpy(mapped, new_str, strlen(new_str));
    printf("修改后文件内容: %s\n", mapped);
    
    // 7. 同步修改到磁盘（可选）
    if (msync(mapped, FILE_SIZE, MS_SYNC) == -1) {
        perror("msync failed");
    }
    
    // 8. 重新读取文件验证修改
    char *buf = malloc(100 * sizeof(char));
    lseek(fd, 0, SEEK_SET);
    read(fd, buf, 50);
    printf("同步后文件内容：%s\n", buf);

    // 9. 清理
    if (munmap(mapped, FILE_SIZE)) {
        perror("munmap failed");
    }
    close(fd);
    unlink(tmpfile);  // 删除临时文件

    printf("mmap test SUCCESS\n");

    return EXIT_SUCCESS;
}