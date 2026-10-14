#import <Foundation/Foundation.h>
#import "Shop/Services/Repo.h"

int main(int argc, char *argv[]) {
    @autoreleasepool {
        Repo *repo = [[Repo alloc] init];
        [repo report];
    }
    return 0;
}
