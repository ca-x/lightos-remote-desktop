#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#include <fcntl.h>
#include <stdio.h>
#include <unistd.h>
#include "mac-keymap.h"
static pid_t owner;
static AXUIElementRef window;
static NSString *title;
static CFAbsoluteTime pulse;
static bool held[256];
static bool focused(void){
    if(NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier!=owner)return false;
    CFTypeRef t=NULL,p=NULL,s=NULL;
    if(AXUIElementCopyAttributeValue(window,kAXTitleAttribute,&t)||AXUIElementCopyAttributeValue(window,kAXPositionAttribute,&p)||AXUIElementCopyAttributeValue(window,kAXSizeAttribute,&s)){if(t)CFRelease(t);if(p)CFRelease(p);if(s)CFRelease(s);return false;}
    CGPoint point;CGSize size;bool ok=AXValueGetValue(p,kAXValueCGPointType,&point)&&AXValueGetValue(s,kAXValueCGSizeType,&size)&&[title isEqualToString:(__bridge NSString*)t];
    CGEventRef e=CGEventCreate(NULL);CGPoint cursor=CGEventGetLocation(e);CFRelease(e);
    ok=ok&&CGRectContainsPoint(CGRectMake(point.x,point.y,size.width,size.height),cursor);CFRelease(t);CFRelease(p);CFRelease(s);return ok;
}
static void emit(unsigned code,bool down){char b[96];int n=snprintf(b,sizeof(b),"{\"type\":\"key\",\"code\":%u,\"pressed\":%s}\n",code,down?"true":"false");if(write(1,b,n)!=n)_exit(0);}
static CGEventRef event(CGEventTapProxy proxy,CGEventType type,CGEventRef e,void *ctx){
    if(type==kCGEventTapDisabledByTimeout||type==kCGEventTapDisabledByUserInput){CFRunLoopStop(CFRunLoopGetMain());return e;}
    if(!focused()||CGEventGetIntegerValueField(e,kCGEventSourceUnixProcessID)!=0)return e;
    unsigned k=(unsigned)CGEventGetIntegerValueField(e,kCGKeyboardEventKeycode),code=k<128?mac_keys[k]:0;
    if(!code)return NULL;bool down=type==kCGEventKeyDown;if(type==kCGEventFlagsChanged)down=CGEventSourceKeyState(kCGEventSourceStateHIDSystemState,k);
    held[code]=down;if(down&&code==1&&(held[29]||held[97])&&(held[56]||held[100])&&(held[42]||held[54])){puts("{\"type\":\"released\",\"reason\":\"escape\"}");CFRunLoopStop(CFRunLoopGetMain());return NULL;}
    if(code==58){emit(code,true);emit(code,false);}else emit(code,down);return NULL;
}
static void tick(CFRunLoopTimerRef t,void *c){char b[64];ssize_t n;while((n=read(0,b,sizeof(b)))>0)pulse=CFAbsoluteTimeGetCurrent();if(n==0||CFAbsoluteTimeGetCurrent()-pulse>1.5||!focused())CFRunLoopStop(CFRunLoopGetMain());}
int main(int argc,char **argv){@autoreleasepool{
    if(argc!=2)return 2;setvbuf(stdout,NULL,_IONBF,0);fcntl(0,F_SETFL,O_NONBLOCK);fcntl(1,F_SETFL,O_NONBLOCK);
    owner=NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier;AXUIElementRef app=AXUIElementCreateApplication(owner);
    if(AXUIElementCopyAttributeValue(app,kAXFocusedWindowAttribute,(CFTypeRef*)&window)){CFRelease(app);puts("{\"type\":\"error\",\"message\":\"Accessibility permission is required on the viewing device\"}");return 3;}CFRelease(app);
    CFTypeRef value=NULL;if(AXUIElementCopyAttributeValue(window,kAXTitleAttribute,&value)){CFRelease(window);return 3;}
    title=[(__bridge NSString*)value copy];CFRelease(value);
    NSString *expected=[NSString stringWithUTF8String:argv[1]];
    if(!title.length||(![title containsString:expected]&&![title containsString:@"LightOS"]&&![title containsString:@"远程"]&&![title containsString:@"Remote"])||!focused()){CFRelease(window);return 3;}
    CGEventMask mask=CGEventMaskBit(kCGEventKeyDown)|CGEventMaskBit(kCGEventKeyUp)|CGEventMaskBit(kCGEventFlagsChanged);
    CFMachPortRef tap=CGEventTapCreate(kCGSessionEventTap,kCGHeadInsertEventTap,kCGEventTapOptionDefault,mask,event,NULL);
    if(!tap){puts("{\"type\":\"error\",\"message\":\"Keyboard capture permission is required on the viewing device\"}");CFRelease(window);return 4;}
    CFRunLoopSourceRef source=CFMachPortCreateRunLoopSource(NULL,tap,0);CFRunLoopAddSource(CFRunLoopGetMain(),source,kCFRunLoopCommonModes);
    pulse=CFAbsoluteTimeGetCurrent();CFRunLoopTimerRef timer=CFRunLoopTimerCreate(NULL,pulse,0.025,0,0,tick,NULL);CFRunLoopAddTimer(CFRunLoopGetMain(),timer,kCFRunLoopCommonModes);
    puts("{\"type\":\"ready\",\"backend\":\"macos\"}");
    for(unsigned k=54;k<=62;k++)if(k!=57&&CGEventSourceKeyState(kCGEventSourceStateHIDSystemState,k)){held[mac_keys[k]]=true;emit(mac_keys[k],true);}
    CFRunLoopRun();CGEventTapEnable(tap,false);CFMachPortInvalidate(tap);CFRelease(timer);CFRelease(source);CFRelease(tap);CFRelease(window);return 0;
}}
