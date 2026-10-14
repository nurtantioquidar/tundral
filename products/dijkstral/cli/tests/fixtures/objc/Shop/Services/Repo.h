@import Foundation;
#import "../Models/Shape.h"

@class Circle;

@interface Repo : NSObject {
    Shape *_last;
}
@property (nonatomic, strong) Circle *favorite;
- (void)add:(Shape *)shape;
- (Shape *)find:(NSString *)name;
- (void)report;
@end
