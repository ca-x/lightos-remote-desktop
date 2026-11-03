#define _GNU_SOURCE
#include <X11/Xlib.h>
#include <X11/XKBlib.h>
#include <stdbool.h>
#include <fcntl.h>
#include <poll.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>
int capture_wayland(const char *title);
static bool held[768];static int xerror;
long long now_ms(void){struct timespec t;clock_gettime(CLOCK_MONOTONIC,&t);return t.tv_sec*1000LL+t.tv_nsec/1000000;}
int heartbeat(long long *pulse){char b[64];ssize_t n;while((n=read(0,b,sizeof(b)))>0)*pulse=now_ms();return n!=0&&now_ms()-*pulse<1500;}
int title_matches(const char *title,const char *expected){return title&&expected&&*expected&&(strstr(title,expected)||strstr(title,"LightOS")||strstr(title,"Remote")||strstr(title,"远程"));}
void key_event(unsigned code,int down){
    if(!code||code>=768)return;held[code]=down;
    if(down&&code==1&&(held[29]||held[97])&&(held[56]||held[100])&&(held[42]||held[54])){puts("{\"type\":\"released\",\"reason\":\"escape\"}");exit(0);}
    char b[96];int n=snprintf(b,sizeof(b),"{\"type\":\"key\",\"code\":%u,\"pressed\":%s}\n",code,down?"true":"false");if(write(1,b,n)!=n)exit(0);
}
static int error_handler(Display *d,XErrorEvent *e){xerror=1;return 0;}
static Window top(Display *d){Window w;int revert;XGetInputFocus(d,&w,&revert);if(w<=PointerRoot)return 0;for(int i=0;i<32;i++){Window root,parent,*children=NULL;unsigned n;if(!XQueryTree(d,w,&root,&parent,&children,&n))return 0;if(children)XFree(children);if(parent==root||!parent)return w;w=parent;}return 0;}
static int x11(const char *expected){
    Display *d=XOpenDisplay(NULL);if(!d)return 3;XSetErrorHandler(error_handler);Window window=top(d);char *title=NULL;
    if(!window||!XFetchName(d,window,&title)||!title_matches(title,expected)){if(title)XFree(title);XCloseDisplay(d);return 3;}XFree(title);
    XkbSetDetectableAutoRepeat(d,True,NULL);if(XGrabKeyboard(d,window,False,GrabModeAsync,GrabModeAsync,CurrentTime)!=GrabSuccess){XCloseDisplay(d);return 4;}
    XFlush(d);puts("{\"type\":\"ready\",\"backend\":\"x11\"}");char keys[32];XQueryKeymap(d,keys);for(int k=8;k<256;k++)if((keys[k/8]>>(k%8))&1)key_event(k-8,1);
    long long pulse=now_ms();while(heartbeat(&pulse)&&!xerror&&top(d)==window){
        char *current=NULL;bool same=XFetchName(d,window,&current)&&title_matches(current,expected);if(current)XFree(current);if(!same)break;
        while(XPending(d)){XEvent e;XNextEvent(d,&e);if((e.type==KeyPress||e.type==KeyRelease)&&!e.xkey.send_event&&e.xkey.keycode>=8)key_event(e.xkey.keycode-8,e.type==KeyPress);}
        struct pollfd f[]={{ConnectionNumber(d),POLLIN,0},{0,POLLIN|POLLHUP,0}};poll(f,2,20);
    }
    XUngrabKeyboard(d,CurrentTime);XFlush(d);XCloseDisplay(d);return 0;
}
int main(int argc,char **argv){
    if(argc!=2||!*argv[1])return 2;setvbuf(stdout,NULL,_IONBF,0);fcntl(0,F_SETFL,O_NONBLOCK);fcntl(1,F_SETFL,O_NONBLOCK);
    if(getenv("WAYLAND_DISPLAY"))return capture_wayland(argv[1]);
    if(getenv("DISPLAY"))return x11(argv[1]);return 3;
}
