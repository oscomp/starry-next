#include <stdio.h>
#include <stdlib.h>
#include <sys/wait.h>
#include <unistd.h>

#define N 150

void forktest(void) {
  int n, pid;

  printf("fork test\n");

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
