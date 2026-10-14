#import <Foundation/Foundation.h>
#import "Util.h"

NS_ASSUME_NONNULL_BEGIN

// NOTE: shapes are value-like; WHY: keep the fixture small.
@protocol Drawable <NSObject>
- (void)draw;
@optional
- (NSString *)caption;
@end

typedef enum {
    ShapeKindCircle,
    ShapeKindSquare
} ShapeKind;

@interface Shape : NSObject <Drawable>
@property (nonatomic, copy) NSString *name;
@property (nonatomic) ShapeKind kind;
@property (nonatomic, strong) NSArray<Shape *> *children;
- (instancetype)initWithName:(NSString *)name;
- (double)area;
- (void)draw;
+ (instancetype)shapeNamed:(NSString *)name;
@end

NS_ASSUME_NONNULL_END
