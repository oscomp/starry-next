#include <stdio.h>
#include <stdlib.h>
#include <sys/wait.h>
#include <unistd.h>
#include <sys/mman.h>

#define N 150
#define LEN (1024 * 1024)

void forktest(void) {
  int n, pid;

  printf("fork test\n");

  char *ptr = mmap(NULL, LEN * sizeof(char), PROT_READ | PROT_WRITE,
                  MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);

  for (n = 0; n < LEN; n++) {
      *(ptr + n) = 'a';
  }

  for (n = 0; n < N; n++) {
    pid = fork();
    if (pid < 0)
      break;
    if (pid == 0)
      exit(0);
  }

  if (n == N) {
    printf("fork test OK\n");
    exit(0);
  }

  for (; n > 0; n--) {
    if (wait(0) < 0) {
      printf("wait stopped early\n");
      exit(1);
    }
  }

  if (wait(0) != -1) {
    printf("wait got too many\n");
    exit(1);
  }
}

int main(void) {
  forktest();
  exit(1);
}
