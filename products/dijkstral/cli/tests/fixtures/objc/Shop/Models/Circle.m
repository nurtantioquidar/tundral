#import "Circle.h"

@implementation Circle

- (double)area {
    return 3.14 * self.radius * self.radius;
}

- (void)draw {
    [super draw];
}

@end
