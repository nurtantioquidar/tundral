#import "Repo.h"
#import "Circle.h"
#import <UIKit/UIKit.h>

@implementation Repo

- (void)add:(Shape *)shape {
    _last = shape;
    [shape draw];
    [_last refresh];
}

- (Shape *)find:(NSString *)name {
    Shape *found = [Shape shapeNamed:name];
    [found area];
    [Shape missing];
    return found;
}

- (void)report {
    Circle *c = [[Circle alloc] init];
    [c area];
    [self.favorite draw];
    [self performSelector:@selector(report)];
    [self find:@"x"];
}

@end

// WHY: C functions in implementation files are not modelled.
static void logAll(Repo *repo) {
    [repo report];
}
