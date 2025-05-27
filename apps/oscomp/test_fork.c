// Test that fork fails gracefully.
// Tiny executable so that the limit can be filling the proc table.
#include <stdio.h>
#include <sys/wait.h>
#include <unistd.h>
#include <stdlib.h>

#define N  1000


void
forktest(void)
{
  int n, pid;

  printf("fork test\n");

  for(n=0; n<N; n++){
    pid = fork();
    if(pid < 0)
      break;
    if(pid == 0)
      exit(0);
  }

  if(n == N){
    printf("fork claimed to work %d times!\n", n);
    exit(1);
  }

  for(; n > 0; n--){
    if(wait(0) < 0){
      printf("wait stopped early\n");
      exit(1);
    }
  }

  if(wait(0) != -1){
    printf("wait got too many\n");
    exit(1);
  }

  printf("fork test OK\n");
}

int
main(void)
{
  forktest();
  exit(0);
}
