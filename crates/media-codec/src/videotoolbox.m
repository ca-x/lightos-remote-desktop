#import <Foundation/Foundation.h>
#import <VideoToolbox/VideoToolbox.h>
#import <CoreVideo/CoreVideo.h>
#import <CoreMedia/CoreMedia.h>
#include <stdlib.h>

typedef void (*Completion)(void *, int32_t, const uint8_t *, size_t);
typedef struct {Completion done;void *context;CVPixelBufferRef image;} Pending;
typedef struct {VTCompressionSessionRef codec;VTPixelTransferSessionRef transfer;CVPixelBufferPoolRef pool;int width,height,fps;int64_t sequence;} Encoder;
static void finish(Pending *pending,OSStatus status,NSData *data) {
    pending->done(pending->context,status,data.bytes,data.length);
    CVPixelBufferRelease(pending->image);free(pending);
}
static void output(void *context,void *frame,OSStatus status,VTEncodeInfoFlags flags,CMSampleBufferRef sample) {
    (void)context;(void)flags;
    @autoreleasepool {
        Pending *pending=frame;
        if(status || !sample){finish(pending,status,nil);return;}
        CMBlockBufferRef block=CMSampleBufferGetDataBuffer(sample);
        size_t size=block?CMBlockBufferGetDataLength(block):0;
        if(!size || size>16*1024*1024){finish(pending,-1,nil);return;}
        NSMutableData *avcc=[NSMutableData dataWithLength:size];
        if(CMBlockBufferCopyDataBytes(block,0,size,avcc.mutableBytes)){finish(pending,-1,nil);return;}
        CMVideoFormatDescriptionRef format=CMSampleBufferGetFormatDescription(sample);
        size_t count=0,parameterSize=0;const uint8_t *parameter=NULL;int length=0;
        if(CMVideoFormatDescriptionGetH264ParameterSetAtIndex(format,0,&parameter,&parameterSize,&count,&length)||length<1||length>4){finish(pending,-1,nil);return;}
        CFArrayRef attachments=CMSampleBufferGetSampleAttachmentsArray(sample,false);
        bool key=true;
        if(attachments&&CFArrayGetCount(attachments)){
            CFDictionaryRef info=CFArrayGetValueAtIndex(attachments,0);
            key=!CFDictionaryContainsKey(info,kCMSampleAttachmentKey_NotSync)||CFDictionaryGetValue(info,kCMSampleAttachmentKey_NotSync)==kCFBooleanFalse;
        }
        NSMutableData *annex=[NSMutableData dataWithCapacity:size+128];const uint8_t start[4]={0,0,0,1};
        if(key)for(size_t i=0;i<count;i++){
            if(CMVideoFormatDescriptionGetH264ParameterSetAtIndex(format,i,&parameter,&parameterSize,NULL,NULL)){finish(pending,-1,nil);return;}
            [annex appendBytes:start length:4];[annex appendBytes:parameter length:parameterSize];
        }
        const uint8_t *bytes=avcc.bytes;size_t at=0;
        while(at<size){
            if(size-at<(size_t)length){finish(pending,-1,nil);return;}uint32_t n=0;
            for(int i=0;i<length;i++)n=(n<<8)|bytes[at++];
            if(n==0||n>size-at){finish(pending,-1,nil);return;}
            [annex appendBytes:start length:4];[annex appendBytes:bytes+at length:n];at+=n;
        }
        finish(pending,0,annex);
    }
}
static OSStatus number(VTSessionRef session,CFStringRef key,int32_t value){
    CFNumberRef n=CFNumberCreate(NULL,kCFNumberSInt32Type,&value);OSStatus s=VTSessionSetProperty(session,key,n);CFRelease(n);return s;
}
int32_t lrd_vt_rate(Encoder *e,uint32_t bitrate,uint32_t fps){
    OSStatus s=number(e->codec,kVTCompressionPropertyKey_AverageBitRate,bitrate);
    if(!s)s=number(e->codec,kVTCompressionPropertyKey_ExpectedFrameRate,fps);
    if(!s)e->fps=fps;
    return s;
}
void lrd_vt_destroy(Encoder *e){
    if(!e)return;
    if(e->codec){VTCompressionSessionCompleteFrames(e->codec,kCMTimeInvalid);VTCompressionSessionInvalidate(e->codec);CFRelease(e->codec);}
    if(e->pool)CFRelease(e->pool);if(e->transfer)CFRelease(e->transfer);free(e);
}
Encoder *lrd_vt_create(uint32_t width,uint32_t height,uint32_t fps,uint32_t bitrate,int32_t *error){
    @autoreleasepool {
        Encoder *e=calloc(1,sizeof(*e));if(!e){*error=-1;return NULL;}e->width=width;e->height=height;
        NSDictionary *spec=@{(__bridge NSString *)kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder:@YES,
            (__bridge NSString *)kVTVideoEncoderSpecification_EnableLowLatencyRateControl:@YES};
        OSStatus s=VTCompressionSessionCreate(NULL,width,height,kCMVideoCodecType_H264,(__bridge CFDictionaryRef)spec,NULL,NULL,output,NULL,&e->codec);
        // LowLatencyRateControl itself enforces High/auto-level, no B frames,
        // no lookahead and an infinite GOP. Setting the fixed dependency
        // properties again is rejected by some Apple encoders (-12900).
        if(s)fprintf(stderr,"VT create failed: %d\n",s);
        if(!s)(void)VTSessionSetProperty(e->codec,kVTCompressionPropertyKey_RealTime,kCFBooleanTrue);
        if(!s){s=lrd_vt_rate(e,bitrate,fps);if(s)fprintf(stderr,"VT rate failed: %d\n",s);}
        if(!s){s=VTCompressionSessionPrepareToEncodeFrames(e->codec);if(s)fprintf(stderr,"VT prepare failed: %d\n",s);}
        CFTypeRef hardware=NULL;
        if(!s){
            OSStatus query=VTSessionCopyProperty(e->codec,kVTCompressionPropertyKey_UsingHardwareAcceleratedVideoEncoder,NULL,&hardware);
            // Apple's low-latency encoder does not expose this optional property
            // on every release. RequireHardwareAcceleratedVideoEncoder above is
            // mandatory selection: creation cannot silently choose software.
            if(query==0&&hardware!=kCFBooleanTrue)s=-1;
            else if(query!=0&&query!=kVTPropertyNotSupportedErr)s=query;
        }
        if(hardware)CFRelease(hardware);
        if(!s){s=VTPixelTransferSessionCreate(NULL,&e->transfer);if(s)fprintf(stderr,"VT transfer failed: %d\n",s);}
        NSDictionary *attributes=@{(__bridge NSString *)kCVPixelBufferWidthKey:@(width),(__bridge NSString *)kCVPixelBufferHeightKey:@(height),
            (__bridge NSString *)kCVPixelBufferPixelFormatTypeKey:@(kCVPixelFormatType_32BGRA),(__bridge NSString *)kCVPixelBufferIOSurfacePropertiesKey:@{}};
        if(!s){s=CVPixelBufferPoolCreate(NULL,NULL,(__bridge CFDictionaryRef)attributes,&e->pool);if(s)fprintf(stderr,"VT pixel pool failed: %d\n",s);}
        if(s){*error=s;lrd_vt_destroy(e);return NULL;}*error=0;return e;
    }
}
void lrd_vt_submit(Encoder *e,CVPixelBufferRef input,bool key,void *context,Completion done){
    @autoreleasepool {
        CVPixelBufferRef image=NULL;OSStatus status=0;
        if(CVPixelBufferGetWidth(input)==(size_t)e->width&&CVPixelBufferGetHeight(input)==(size_t)e->height)image=CVPixelBufferRetain(input);
        else {
            status=CVPixelBufferPoolCreatePixelBuffer(NULL,e->pool,&image);
            if(!status)status=VTPixelTransferSessionTransferImage(e->transfer,input,image);
        }
        if(status){if(image)CVPixelBufferRelease(image);done(context,status,NULL,0);return;}
        Pending *pending=calloc(1,sizeof(*pending));if(!pending){CVPixelBufferRelease(image);done(context,-1,NULL,0);return;}
        pending->image=image;pending->done=done;pending->context=context;
        NSDictionary *options=key?@{(__bridge NSString *)kVTEncodeFrameOptionKey_ForceKeyFrame:@YES}:nil;
        status=VTCompressionSessionEncodeFrame(e->codec,image,CMTimeMake(e->sequence++,e->fps),kCMTimeInvalid,(__bridge CFDictionaryRef)options,pending,NULL);
        if(status)finish(pending,status,nil);
    }
}
CVPixelBufferRef lrd_vt_rgba(const uint8_t *rgba,uint32_t width,uint32_t height){
    @autoreleasepool {
        CVPixelBufferRef image=NULL;NSDictionary *attrs=@{(__bridge NSString *)kCVPixelBufferIOSurfacePropertiesKey:@{}};
        if(CVPixelBufferCreate(NULL,width,height,kCVPixelFormatType_32BGRA,(__bridge CFDictionaryRef)attrs,&image))return NULL;
        if(CVPixelBufferLockBaseAddress(image,0)){CVPixelBufferRelease(image);return NULL;}
        size_t stride=CVPixelBufferGetBytesPerRow(image);uint8_t *target=CVPixelBufferGetBaseAddress(image);
        for(size_t y=0;y<height;y++)for(size_t x=0;x<width;x++){const uint8_t *s=rgba+(y*width+x)*4;uint8_t *d=target+y*stride+x*4;d[0]=s[2];d[1]=s[1];d[2]=s[0];d[3]=s[3];}
        CVPixelBufferUnlockBaseAddress(image,0);return image;
    }
}
double lrd_capture_age(CMSampleBufferRef sample){
    CMTime delta=CMTimeSubtract(CMClockGetTime(CMClockGetHostTimeClock()),CMSampleBufferGetPresentationTimeStamp(sample));
    return CMTimeGetSeconds(delta);
}
