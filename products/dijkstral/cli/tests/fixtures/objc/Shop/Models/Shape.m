#import "Shape.h"

@interface Shape ()
@property (nonatomic, copy) NSString *secret;
- (void)refresh;
@end

@implementation Shape

- (instancetype)initWithName:(NSString *)name {
    self = [super init];
    if (self) {
        _name = [name copy];
        [self refresh];
    }
    return self;
}

- (double)area {
    return 0;
}

- (void)refresh {
    double a = self.area;
    NSLog(@"%f", a);
}

- (void)draw {
    [self refresh];
}

+ (instancetype)shapeNamed:(NSString *)name {
    return [[self alloc] initWithName:name];
}

@end
