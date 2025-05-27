#include <stdio.h>
#include <sys/wait.h>
#include <unistd.h>
#include <sys/mman.h>
#include <stdlib.h>

#define N 100

void test() {
    int num_size = (10 * 1024 * 1024) / sizeof(int);

    int *ptr = mmap(NULL, sizeof(int) * num_size, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    int i = 0;
    for (i = 0; i < num_size; i++) {
        *(ptr + i) = i;
    }

    int pid;
    for (i = 0 ; i < N; i++) {
        pid = fork();
        if (pid == 0) {
            printf("i = %d sleep ....\n", i);

            sleep(5);
            exit(0);
        } else if (pid < 0) {
            exit(1);
        }
    }

    for (i = 0; i < N; i++) {
        wait(0);
    }

    printf("pass!!!\n");
    exit(0);
}

int main() {
    test();
}
