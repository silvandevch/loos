/* The root class NSObject and the runtime's own classes (block classes, Protocol). Compiled as
 * Objective-C (manual reference counting) into Prism's runtime. */
#include "objc.h"

typedef struct _NSZone NSZone;
typedef unsigned long NSUInteger;

__attribute__((objc_root_class))
@interface NSObject { Class isa; }
@end

@implementation NSObject
+ (void)load {}
+ (void)initialize {}
+ (id)alloc { return class_createInstance(self, 0); }
+ (id)allocWithZone:(NSZone *)z { (void)z; return class_createInstance(self, 0); }
+ (id)new { return [[self alloc] init]; }
+ (Class)class { return self; }
+ (Class)superclass { return class_getSuperclass(self); }
+ (BOOL)instancesRespondToSelector:(SEL)s { return class_respondsToSelector(self, s); }
+ (BOOL)isSubclassOfClass:(Class)c { for (Class k = self; k; k = class_getSuperclass(k)) if (k == c) return YES; return NO; }
+ (BOOL)resolveClassMethod:(SEL)s { (void)s; return NO; }
+ (BOOL)resolveInstanceMethod:(SEL)s { (void)s; return NO; }
+ (IMP)instanceMethodForSelector:(SEL)s { return class_getMethodImplementation(self, s); }
+ (BOOL)conformsToProtocol:(void *)p { for (Class k = self; k; k = class_getSuperclass(k)) if (class_conformsToProtocol(k, p)) return YES; return NO; }
+ (id)copyWithZone:(NSZone *)z { (void)z; return self; }
+ (id)mutableCopyWithZone:(NSZone *)z { (void)z; return self; }
- (id)init { return self; }
- (void)dealloc { _prism_objc_dispose(self); }
- (void)finalize {}
- (id)retain { return _prism_objc_rootRetain(self); }
- (oneway void)release { if (_prism_objc_rootRelease(self)) [self dealloc]; }
- (id)autorelease { extern id _prism_objc_rootAutorelease(id); return _prism_objc_rootAutorelease(self); }
- (NSUInteger)retainCount { return _prism_objc_rootRetainCount(self); }
- (BOOL)_tryRetain { [self retain]; return YES; }
- (BOOL)_isDeallocating { return NO; }
- (BOOL)allowsWeakReference { return YES; }
- (BOOL)retainWeakReference { return YES; }
- (Class)class { return object_getClass(self); }
- (Class)superclass { return class_getSuperclass(object_getClass(self)); }
- (id)self { return self; }
- (NSZone *)zone { return 0; }
- (BOOL)isProxy { return NO; }
- (BOOL)isKindOfClass:(Class)c { for (Class k = object_getClass(self); k; k = class_getSuperclass(k)) if (k == c) return YES; return NO; }
- (BOOL)isMemberOfClass:(Class)c { return object_getClass(self) == c; }
- (BOOL)respondsToSelector:(SEL)s { return class_respondsToSelector(object_getClass(self), s); }
- (BOOL)conformsToProtocol:(void *)p { for (Class k = object_getClass(self); k; k = class_getSuperclass(k)) if (class_conformsToProtocol(k, p)) return YES; return NO; }
- (NSUInteger)hash { return (NSUInteger)self; }
- (BOOL)isEqual:(id)o { return self == o; }
- (IMP)methodForSelector:(SEL)s { return class_getMethodImplementation(object_getClass(self), s); }
- (id)performSelector:(SEL)s { return ((id(*)(id, SEL))objc_msgSend)(self, s); }
- (id)performSelector:(SEL)s withObject:(id)a { return ((id(*)(id, SEL, id))objc_msgSend)(self, s, a); }
- (id)performSelector:(SEL)s withObject:(id)a withObject:(id)b { return ((id(*)(id, SEL, id, id))objc_msgSend)(self, s, a, b); }
- (id)copy { return [(id)self copyWithZone:0]; }
- (id)mutableCopy { return [(id)self mutableCopyWithZone:0]; }
- (id)forwardingTargetForSelector:(SEL)s { (void)s; return nil; }
- (void)doesNotRecognizeSelector:(SEL)s { _prism_objc_unrecognized(self, s); }
@end

/* Methods the compiler sends that NSObject's interface above does not declare. */
@interface NSObject (Prism)
- (id)copyWithZone:(NSZone *)z;
- (id)mutableCopyWithZone:(NSZone *)z;
@end

@interface Protocol : NSObject @end
@implementation Protocol
- (id)retain { return self; }
- (oneway void)release {}
- (id)autorelease { return self; }
@end

/* Blocks: the isa words _NSConcrete*Block become copies of these classes (objc.c). */
@interface __NSBlock : NSObject @end
@implementation __NSBlock
- (id)copy { return _Block_copy(self); }
- (id)copyWithZone:(NSZone *)z { (void)z; return _Block_copy(self); }
- (id)retain { return _Block_copy(self); }
- (oneway void)release { _Block_release(self); }
- (id)autorelease { extern id _prism_objc_rootAutorelease(id); return _prism_objc_rootAutorelease(self); }
- (NSUInteger)retainCount { return 1; }
@end
@interface __NSStackBlock__ : __NSBlock @end
@implementation __NSStackBlock__ @end
@interface __NSMallocBlock__ : __NSBlock @end
@implementation __NSMallocBlock__ @end
@interface __NSGlobalBlock__ : __NSBlock @end
@implementation __NSGlobalBlock__
- (id)retain { return self; }
- (oneway void)release {}
@end
@interface __NSAutoBlock__ : __NSBlock @end
@implementation __NSAutoBlock__ @end
@interface __NSFinalizingBlock__ : __NSBlock @end
@implementation __NSFinalizingBlock__ @end
@interface __NSBlockVariable__ : NSObject @end
@implementation __NSBlockVariable__ @end
