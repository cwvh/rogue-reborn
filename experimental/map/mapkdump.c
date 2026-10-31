// mapkdump [--full] MAP...: every field of each map read by mapk_read, as `mapdump --full` prints
// it. A map that fails to read gets "MAP: failed" on stderr.
#include"mapk.h"
#include<stdio.h>
#include<stdlib.h>
#include<string.h>
#ifdef _WIN32
#include<fcntl.h>
#include<io.h>
#endif
typedef uint32_t U;typedef uint16_t H;typedef uint8_t C;typedef int64_t J;
#define O printf
#define DO(n,...) {J i=0,_n=(n);for(;i<_n;++i){__VA_ARGS__;}}

static U*w;
static void f(U x){O(" %08x",x);}
static void fs(U*x,J n){DO(n,f(x[i]))}
static void s(U x){putchar(' ');putchar('"');for(C*c=(C*)(w+x);*c;c++)*c=='"'||*c=='\\'?O("\\%c",*c):*c<32||*c>126?O("\\x%02x",*c):putchar(*c);putchar('"');}
static void ss(U*x,J n){DO(n,s(x[i]))}
static void L(U n,U x){O(" %u",n);ss(w+x,n);}
static void sec(char*k,U*r,U n){O("%s %u",k,r[1]);s(r[3]);O(" %u\n",n);}
static void cm(U*c){O("common");fs(c,12);s(c[12]);O(" %u 4",c[13]);ss(c+14,11);O("\n");}

static void dy(U*y){U*k=w+y[17];O("dynamic %u",y[1]);s(y[3]);s(y[4]);fs(y+5,12);O("\n");switch(y[1]){
 case 14:cm(k);if(k[25])DO(k[25],U*p=w+k[26]+i*wK;O("param");s(p[0]);fs(p+1,9);O(" %u %u\n",p[10],p[11]))
  else{U*f=w+k[26];O("flat");L(f[0],f[1]);fs(f+2,4);O("\n");}break;
 case 15:cm(k);O("animation %u",k[25]);L(k[26],k[27]);fs(k+28,3);O(" %u",k[31]);ss(k+32,3);fs(k+35,5);O("\n");break;
 case 16:cm(k);O("touchplate %u",k[25]);L(k[26],k[27]);fs(k+28,3);L(k[31],k[32]);ss(k+33,3);fs(k+36,5);O("\n");break;
 case 20:O("glass");s(k[0]);O("\n");break;
 case 25:O("once");ss(k,2);fs(k+2,6);L(k[8],k[9]);O("\n");break;
 case 31:DO(k[0],U*h=w+k[1]+i*wH;O("halo");s(h[0]);fs(h+1,8);O("\n"))}}

static void ob(U*o){O("object %u",o[1]);s(o[3]);O(" %u",o[5]);s(o[7]);O("\n");
 DO(o[8],O("v");fs(w+o[9]+3*i,3);O("\n"))
 DO(o[10],U*d=w+o[11]+i*wD;O("data %u %u %u\n",d[0],d[1],d[5]);
  DO(d[1],H*a=(H*)(w+d[3])+3*i,*b=(H*)(w+d[4])+3*i;O("face");fs(w+d[2]+4*i,4);O(" %u %u %u %u %u %u\n",a[0],a[1],a[2],b[0],b[1],b[2]))
  DO(d[5],O("tv");fs(w+d[6]+3*i,3);fs(w+d[7]+2*i,2);fs(w+d[8]+4*i,4);O("\n")))
 DO(o[12],O("cv");fs(w+o[13]+3*i,3);O("\n"))
 DO(o[14],O("cp");fs(w+o[15]+4*i,4);O("\n"))
 DO(o[16],H*t=(H*)(w+o[17])+8*i;O("tag");DO(8,O(" %u",t[i]))O("\n"))
 DO(o[18],U*g=w+o[19]+4*i;H*x=(H*)(w+g[3]);O("group");s(g[0]);O(" %u %u",g[1],g[2]);DO(g[2],O(" %u",x[i]))O("\n"))}

static void rm(U*q){O("room %u",q[1]);s(q[3]);O(" %u",q[4]);~q[5]?O(" %u",q[5]):O(" -");O(" %u",q[6]);
 ~q[7]?O(" %u",q[7]):O(" -");~q[7]&&q[7]?fs(q+8,6):(void)O(" -");O("\n");
 DO(q[14],U*v=w+q[15]+i*wV;O("level");s(v[0]);O(" %u %u",v[5],v[3]);fs(w+v[4],v[3]);O("\n");DO(v[1],O("box");fs(w+v[2]+18*i,18);O("\n")))
 DO(q[16],O("floor");fs(w+q[17]+2*i,2);O("\n"))
 O("unknown %u",q[18]);fs(w+q[19],q[18]);O("\n");}

static void full(){U*r=w;O("timestamp %u\n",r[0]);
 O("materials %u %u\n",r[2],r[5]);DO(r[5],U*t=w+r[6]+i*wM;O("material %u",t[1]);s(t[3]);s(t[4]);f(t[5]);O(" %u %d",t[6],t[7]);fs(t+8,13);O(" %d\n",t[21]==1))
 O("geometries %u %u\n",r[8],r[11]);DO(r[11],ob(w+r[12]+i*wO))
 sec("portals",r+13,r[17]);DO(r[17],U*t=w+r[18]+i*wP;O("portal %u",t[1]);s(t[3]);O(" %u %u %u",t[6],t[7],t[4]);fs(w+t[5],3*t[4]);O("\n"))
 sec("lights",r+19,r[23]);
 sec("dynamic_objects",r+24,r[28]);DO(r[28],dy(w+r[29]+i*wY))
 sec("rooms",r+30,r[34]);DO(r[34],rm(w+r[35]+i*wQ))
 sec("transitions",r+36,r[40]);DO(r[40],U*t=w+r[41]+i*wT;O("transition");s(t[0]);fs(t+1,6);O("\n"))
 sec("planning_levels",r+42,r[46]);DO(r[46],U*t=w+r[47]+i*wL;O("planning");fs(t,2);L(t[2],t[3]);O("\n"))}

int main(int n,char**v){int st=0;J m=8<<20;
#ifdef _WIN32
 _setmode(_fileno(stdout),_O_BINARY);_setmode(_fileno(stderr),_O_BINARY);
#endif
 if(!(w=malloc(4*m)))return fprintf(stderr,"mapkdump: out of memory\n"),1;
 DO(n-1,char*a=v[i+1];if(strcmp(a,"--full")){if(mapk_read(a,w,m))full();else fprintf(stderr,"%s: failed\n",a),st=1;})
 return n<2?fprintf(stderr,"usage: mapkdump [--full] MAP...\n"),2:st;}
