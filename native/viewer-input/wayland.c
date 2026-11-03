#define _GNU_SOURCE
#include <wayland-client.h>
#include <sys/mman.h>
#include <poll.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include "keyboard-shortcuts-inhibit-unstable-v1-client-protocol.h"
#include "wlr-layer-shell-unstable-v1-client-protocol.h"
#include "wlr-foreign-toplevel-management-unstable-v1-client-protocol.h"
extern long long now_ms(void);extern int heartbeat(long long*);extern void key_event(unsigned,int);extern int title_matches(const char*,const char*);
typedef struct {struct zwlr_foreign_toplevel_handle_v1 *handle;char *title;} Top;
static struct {
 struct wl_display *display;struct wl_compositor *compositor;struct wl_shm *shm;struct wl_seat *seat;struct wl_keyboard *keyboard;
 struct zwlr_layer_shell_v1 *layers;struct zwp_keyboard_shortcuts_inhibit_manager_v1 *inhibit;struct zwlr_foreign_toplevel_manager_v1 *tops;
 struct zwlr_layer_surface_v1 *layer;struct zwp_keyboard_shortcuts_inhibitor_v1 *inhibitor;struct wl_surface *surface;struct wl_buffer *buffer;
 Top *owner,*active;bool capturing,entered,inhibited,ready,quit;
} W;
static void ready(void){if(W.entered&&W.inhibited&&!W.ready){W.ready=true;puts("{\"type\":\"ready\",\"backend\":\"wayland\"}");}}
static void top_title(void *p,struct zwlr_foreign_toplevel_handle_v1 *h,const char *title){Top *t=p;if(W.capturing&&t==W.owner&&t->title&&strcmp(t->title,title))W.quit=true;free(t->title);t->title=strdup(title);}
static void top_app(void *p,struct zwlr_foreign_toplevel_handle_v1 *h,const char *app){}
static void top_output(void *p,struct zwlr_foreign_toplevel_handle_v1 *h,struct wl_output *o){}
static void top_state(void *p,struct zwlr_foreign_toplevel_handle_v1 *h,struct wl_array *states){uint32_t *s;wl_array_for_each(s,states)if(*s==ZWLR_FOREIGN_TOPLEVEL_HANDLE_V1_STATE_ACTIVATED){W.active=p;if(W.capturing&&p!=W.owner)W.quit=true;}}
static void top_done(void *p,struct zwlr_foreign_toplevel_handle_v1 *h){}
static void top_closed(void *p,struct zwlr_foreign_toplevel_handle_v1 *h){Top *t=p;if(t==W.owner)W.quit=true;if(t==W.active)W.active=NULL;zwlr_foreign_toplevel_handle_v1_destroy(h);free(t->title);free(t);}
static void top_parent(void *p,struct zwlr_foreign_toplevel_handle_v1 *h,struct zwlr_foreign_toplevel_handle_v1 *parent){}
static const struct zwlr_foreign_toplevel_handle_v1_listener top_listener={top_title,top_app,top_output,top_output,top_state,top_done,top_closed,top_parent};
static void new_top(void *p,struct zwlr_foreign_toplevel_manager_v1 *m,struct zwlr_foreign_toplevel_handle_v1 *h){Top *t=calloc(1,sizeof(*t));t->handle=h;zwlr_foreign_toplevel_handle_v1_add_listener(h,&top_listener,t);}
static void tops_finished(void *p,struct zwlr_foreign_toplevel_manager_v1 *m){W.quit=true;}
static const struct zwlr_foreign_toplevel_manager_v1_listener tops_listener={new_top,tops_finished};
static void keymap(void *p,struct wl_keyboard *k,uint32_t format,int fd,uint32_t size){close(fd);}
static void enter(void *p,struct wl_keyboard *k,uint32_t serial,struct wl_surface *s,struct wl_array *keys){W.entered=true;ready();uint32_t *key;wl_array_for_each(key,keys)key_event(*key,1);}
static void leave(void *p,struct wl_keyboard *k,uint32_t serial,struct wl_surface *s){W.quit=true;}
static void key(void *p,struct wl_keyboard *k,uint32_t serial,uint32_t time,uint32_t code,uint32_t state){key_event(code,state==WL_KEYBOARD_KEY_STATE_PRESSED);}
static void modifiers(void *p,struct wl_keyboard *k,uint32_t serial,uint32_t depressed,uint32_t latched,uint32_t locked,uint32_t group){}
static void repeat(void *p,struct wl_keyboard *k,int32_t rate,int32_t delay){}
static const struct wl_keyboard_listener keys_listener={keymap,enter,leave,key,modifiers,repeat};
static void caps(void *p,struct wl_seat *s,uint32_t c){if((c&WL_SEAT_CAPABILITY_KEYBOARD)&&!W.keyboard){W.keyboard=wl_seat_get_keyboard(s);wl_keyboard_add_listener(W.keyboard,&keys_listener,NULL);}}
static void seat_name(void *p,struct wl_seat *s,const char *name){}
static const struct wl_seat_listener seat_listener={caps,seat_name};
static void global(void *p,struct wl_registry *r,uint32_t id,const char *name,uint32_t version){
 if(!strcmp(name,"wl_compositor"))W.compositor=wl_registry_bind(r,id,&wl_compositor_interface,version<4?version:4);
 else if(!strcmp(name,"wl_shm"))W.shm=wl_registry_bind(r,id,&wl_shm_interface,1);
 else if(!strcmp(name,"wl_seat")&&!W.seat){W.seat=wl_registry_bind(r,id,&wl_seat_interface,version<5?version:5);wl_seat_add_listener(W.seat,&seat_listener,NULL);}
 else if(!strcmp(name,"zwlr_layer_shell_v1"))W.layers=wl_registry_bind(r,id,&zwlr_layer_shell_v1_interface,1);
 else if(!strcmp(name,"zwp_keyboard_shortcuts_inhibit_manager_v1"))W.inhibit=wl_registry_bind(r,id,&zwp_keyboard_shortcuts_inhibit_manager_v1_interface,1);
 else if(!strcmp(name,"zwlr_foreign_toplevel_manager_v1")){W.tops=wl_registry_bind(r,id,&zwlr_foreign_toplevel_manager_v1_interface,version<3?version:3);zwlr_foreign_toplevel_manager_v1_add_listener(W.tops,&tops_listener,NULL);}
}
static void removed(void *p,struct wl_registry *r,uint32_t id){}
static const struct wl_registry_listener registry_listener={global,removed};
static void configure(void *p,struct zwlr_layer_surface_v1 *l,uint32_t serial,uint32_t width,uint32_t height){
 zwlr_layer_surface_v1_ack_configure(l,serial);if(!W.buffer){int fd=memfd_create("lightos-viewer-input",MFD_CLOEXEC);if(fd<0||ftruncate(fd,4)<0){W.quit=true;return;}struct wl_shm_pool *pool=wl_shm_create_pool(W.shm,fd,4);W.buffer=wl_shm_pool_create_buffer(pool,0,1,1,4,WL_SHM_FORMAT_ARGB8888);wl_shm_pool_destroy(pool);close(fd);}
 wl_surface_attach(W.surface,W.buffer,0,0);wl_surface_damage(W.surface,0,0,1,1);wl_surface_commit(W.surface);
}
static void layer_closed(void *p,struct zwlr_layer_surface_v1 *l){W.quit=true;}
static const struct zwlr_layer_surface_v1_listener layer_listener={configure,layer_closed};
static void inhibited(void *p,struct zwp_keyboard_shortcuts_inhibitor_v1 *i){W.inhibited=true;ready();}
static void uninhibited(void *p,struct zwp_keyboard_shortcuts_inhibitor_v1 *i){W.quit=true;}
static const struct zwp_keyboard_shortcuts_inhibitor_v1_listener inhibitor_listener={inhibited,uninhibited};
int capture_wayland(const char *expected){
 W.display=wl_display_connect(NULL);if(!W.display)return 3;struct wl_registry *r=wl_display_get_registry(W.display);wl_registry_add_listener(r,&registry_listener,NULL);wl_display_roundtrip(W.display);wl_display_roundtrip(W.display);
 if(!W.compositor||!W.shm||!W.seat||!W.layers||!W.inhibit||!W.tops||!W.active||!title_matches(W.active->title,expected)){puts("{\"type\":\"error\",\"message\":\"Focused-window keyboard capture is unavailable on this Wayland compositor\"}");wl_display_disconnect(W.display);return 4;}
 W.owner=W.active;W.capturing=true;W.surface=wl_compositor_create_surface(W.compositor);struct wl_region *empty=wl_compositor_create_region(W.compositor);wl_surface_set_input_region(W.surface,empty);wl_region_destroy(empty);
 W.layer=zwlr_layer_shell_v1_get_layer_surface(W.layers,W.surface,NULL,ZWLR_LAYER_SHELL_V1_LAYER_OVERLAY,"lightos-viewer-input");zwlr_layer_surface_v1_set_size(W.layer,1,1);zwlr_layer_surface_v1_set_keyboard_interactivity(W.layer,ZWLR_LAYER_SURFACE_V1_KEYBOARD_INTERACTIVITY_EXCLUSIVE);zwlr_layer_surface_v1_add_listener(W.layer,&layer_listener,NULL);
 W.inhibitor=zwp_keyboard_shortcuts_inhibit_manager_v1_inhibit_shortcuts(W.inhibit,W.surface,W.seat);zwp_keyboard_shortcuts_inhibitor_v1_add_listener(W.inhibitor,&inhibitor_listener,NULL);
 puts("{\"type\":\"acquiring\",\"backend\":\"wayland\"}");wl_surface_commit(W.surface);long long pulse=now_ms(),start=pulse;
 while(!W.quit&&heartbeat(&pulse)){
  if(!W.ready&&now_ms()-start>1200)break;
  while(wl_display_prepare_read(W.display)!=0)if(wl_display_dispatch_pending(W.display)<0){W.quit=true;break;}
  if(W.quit)break;wl_display_flush(W.display);struct pollfd f[]={{wl_display_get_fd(W.display),POLLIN,0},{0,POLLIN|POLLHUP,0}};int n=poll(f,2,20);
  if(n>0&&(f[0].revents&POLLIN)){if(wl_display_read_events(W.display)<0)break;}else wl_display_cancel_read(W.display);
  if(f[0].revents&(POLLHUP|POLLERR))break;if(wl_display_dispatch_pending(W.display)<0)break;
 }
 zwp_keyboard_shortcuts_inhibitor_v1_destroy(W.inhibitor);zwlr_layer_surface_v1_destroy(W.layer);wl_surface_destroy(W.surface);wl_display_flush(W.display);wl_display_disconnect(W.display);return 0;
}
