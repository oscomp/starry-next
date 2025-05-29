#include <sys/mman.h>  
#include <stdio.h>  
#include <stdlib.h>  
#include <unistd.h>  
#include <string.h>  
#include <errno.h>  
  
// 大页相关常量定义  
#define MAP_HUGETLB     0x40000  
#define MAP_HUGE_SHIFT  26  
#define MAP_HUGE_MASK   0x3f  
#define MAP_HUGE_2MB    (21 << MAP_HUGE_SHIFT)  
#define MAP_HUGE_1GB    (30 << MAP_HUGE_SHIFT)  
  
// 页面大小常量  
#define PAGE_SIZE_4K    (4 * 1024)  
#define PAGE_SIZE_2M    (2 * 1024 * 1024)  
#define PAGE_SIZE_1G    (1024 * 1024 * 1024)  
  
void test_hugepage_allocation() {  
    printf("=== 大页分配测试开始 ===\n");  
      
    // 测试 1: 分配 2MB 大页  
    printf("测试 1: 分配 2MB 大页\n");  
    size_t size_2m = PAGE_SIZE_2M;  
    void *addr_2m = mmap(NULL, size_2m,   
                         PROT_READ | PROT_WRITE,  
                         MAP_PRIVATE | MAP_ANONYMOUS | MAP_HUGETLB | MAP_HUGE_2MB,  
                         -1, 0);  
      
    if (addr_2m == MAP_FAILED) {  
        printf("  2MB 大页分配失败: %s\n", strerror(errno));  
    } else {  
        printf("  2MB 大页分配成功，地址: %p\n", addr_2m);  
          
        // 测试写入数据  
        memset(addr_2m, 0xAA, 1024);  
        printf("  数据写入测试通过\n");  
          
        // 释放内存  
        if (munmap(addr_2m, size_2m) == 0) {  
            printf("  2MB 大页释放成功\n");  
        } else {  
            printf("  2MB 大页释放失败: %s\n", strerror(errno));  
        }  
    }  
      
    // // 测试 2: 分配 1GB 大页  
    // printf("\n测试 2: 分配 1GB 大页\n");  
    // size_t size_1g = PAGE_SIZE_1G;  
    // void *addr_1g = mmap(NULL, size_1g,  
    //                      PROT_READ | PROT_WRITE,  
    //                      MAP_PRIVATE | MAP_ANONYMOUS | MAP_HUGETLB | MAP_HUGE_1GB,  
    //                      -1, 0);  
      
    // if (addr_1g == MAP_FAILED) {  
    //     printf("  1GB 大页分配失败: %s\n", strerror(errno));  
    // } else {  
    //     printf("  1GB 大页分配成功，地址: %p\n", addr_1g);  
          
    //     // 测试写入数据  
    //     memset(addr_1g, 0xBB, 1024);  
    //     printf("  数据写入测试通过\n");  
          
    //     // 释放内存  
    //     if (munmap(addr_1g, size_1g) == 0) {  
    //         printf("  1GB 大页释放成功\n");  
    //     } else {  
    //         printf("  1GB 大页释放失败: %s\n", strerror(errno));  
    //     }  
    // }  
      
    // 测试 3: 默认大页分配（应该使用 2MB）  
    printf("\n测试 3: 默认大页分配\n");  
    size_t size_default = PAGE_SIZE_2M;  
    void *addr_default = mmap(NULL, size_default,  
                              PROT_READ | PROT_WRITE,  
                              MAP_PRIVATE | MAP_ANONYMOUS | MAP_HUGETLB,  
                              -1, 0);  
      
    if (addr_default == MAP_FAILED) {  
        printf("  默认大页分配失败: %s\n", strerror(errno));  
    } else {  
        printf("  默认大页分配成功，地址: %p\n", addr_default);  
          
        // 测试读写  
        *(int*)addr_default = 0x12345678;  
        if (*(int*)addr_default == 0x12345678) {  
            printf("  读写测试通过\n");  
        } else {  
            printf("  读写测试失败\n");  
        }  
          
        // 释放内存  
        if (munmap(addr_default, size_default) == 0) {  
            printf("  默认大页释放成功\n");  
        } else {  
            printf("  默认大页释放失败: %s\n", strerror(errno));  
        }  
    }  
      
    printf("=== 大页分配测试结束 ===\n");  
}  
  
int main() {  
    test_hugepage_allocation();  
    return 0;  
}
