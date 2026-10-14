#import "Services/Repo.h"
#include <vector>

class Native {
public:
    int count;
};

@interface Bridge : NSObject
- (void)run;
@end

@implementation Bridge
- (void)run {
    Repo *repo = [Repo new];
    [repo report];
}
@end
