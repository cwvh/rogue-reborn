// mapki.c: reads a .MAP by interpreting the schemas of mapk.h
#include"mapk.h"
#include<setjmp.h>
#include<stdio.h>
#include<string.h>
typedef uint32_t U;typedef uint8_t C;typedef int64_t J;typedef const char*S;
#define R return
#define DO(n,...) {J i=0,_n=(n);for(;i<_n;++i){__VA_ARGS__;}}

// the file p..e, the map w[0..m) with o used, the last string's length l
static C*p,*e;static U*w,o,m,l;static jmp_buf jb;
static void X(J b){if(b)longjmp(jb,1);}
static C*t(J n){X(n>e-p);C*r=p;p+=n;R r;}
static U y(){R*t(1);}
static U u(){U r;memcpy(&r,t(4),4);R r;}
static U a(J n){X(n>m-o);U r=o;o+=n;R r;}
static U s(){U n=u();X(!n);C*q=t(n);U k=(n+3)/4,r=a(k);w[r+k-1]=0;memcpy(w+r,q,l=n-1);R r;}
static int v8(C*s){for(int n;*s;){C h=*s++;n=h<128?0:(h&224)==192?1:(h&240)==224?2:(h&248)==240?3:-1;
 if(n<0||n==1&&h<194||n==3&&h>244)R 0;U c=h&63>>n;DO(n,if((*s&192)!=128)R 0;c=c<<6|*s++&63)
 if(n==2&&(c<2048||c-55296<2048)||n==3&&(c<65536||c>1114111))R 0;}R 1;}
static U z(){U r=s();X(!v8((C*)(w+r)));R r;}
static void h(U*q){q[0]=u();q[1]=0;q[2]=s();if(l==7&&!memcmp(w+q[2],"Version",7))q[1]=u(),q[2]=s();}
static void g(S x){U n=u();X(!n);C*q=t(n);X(n-1!=strlen(x)||memcmp(q,x,n-1));}

static S Q[128]={['M']=K_M,['O']=K_O,['D']=K_D,['G']=K_G,['P']=K_P,['Y']=K_Y,['Q']=K_Q,['V']=K_V,
 ['T']=K_T,['L']=K_L,['K']=K_K,['F']=K_F,['H']=K_H},
 Y[37]={[14]=K_14,[15]=K_15,[16]=K_16,[20]=K_20,[25]=K_25,[31]=K_31,[36]=K_36};
static U W(S x){U n=0,k;for(;*x;x++){k=0;while(*x>='0'&&*x<='9')k=k*10+*x++-'0';
 n+=strchr("ufs",*x)?k+!k:*x=='S'||*x=='R'?4:*x=='e'||*x=='d'?2:*x=='w'?7:*x=='<'||*x=='>'?0:(x+=*x=='@',1);}R n;}

static void r(S x,U*q);
static U rs(S x,J n){U k=W(x),i0=a(n*k);DO(n,r(x,w+i0+i*k))R i0;}
static void r(S x,U*q){U*q0=q,c=0,k,i0;for(;*x;){k=0;while(*x>='0'&&*x<='9')k=k*10+*x++-'0';switch(*x++){
 case'u':case'f':k+=!k;memcpy(q,t(4*k),4*k);q+=k;break;
 case'b':*q++=y();break;
 case's':k+=!k;DO(k,*q++=s())break;
 case'z':*q++=z();break;
 case'S':*q++=u();h(q);q+=3;break;
 case'R':*q++=0;h(q);q+=3;break;
 case'n':*q++=c=u();break;
 case'#':{J n=(J)c*k;*q++=i0=a(n);memcpy(w+i0,t(4*n),4*n);}break;
 case'%':{J n=(J)c*k*2,k4=(n+3)/4;*q++=i0=a(k4);if(k4)w[i0+k4-1]=0;memcpy(w+i0,t(n),n);}break;
 case'$':*q++=i0=a(c);DO(c,w[i0+i]=s())break;
 case'@':*q++=rs(Q[(C)*x++],c);break;
 case'a':k=*q++=u();X(k>3||k==2);break;
 case'e':k=*q++=y();if(k)X(k>1),*q++=-1;else X((*q++=y())>1);break;
 case'w':k=q0[2]>1?y():-1;*q++=k;memset(q,0,24);if(k+1&&k)memcpy(q,t(24),24);q+=6;break;
 case'!':X(*q++=y());break;
 case'?':k=q0[1];X(k>36||!Y[k]);*q++=rs(Y[k],1);break;
 case'd':k=*q++=u();*q++=k?rs(Q['K'],k):rs(Q['F'],1);break;
 case'<':g("BeginMapv2.1");break;
 case'>':g("EndMap");break;}}}

U mapk_parse(const void*d,ptrdiff_t n,U*v,ptrdiff_t M){p=(C*)d;e=p+n;w=v;o=0;m=M<0?0:M>UINT32_MAX?UINT32_MAX:M;
 if(setjmp(jb))R 0;rs(K_MAP,1);R o;}
U mapk_read(const char*f,U*v,ptrdiff_t M){FILE*F=fopen(f,"rb");if(!F)R 0;J n=fseek(F,0,SEEK_END)?-1:ftell(F);
 if(n<0||n>4*M||fseek(F,0,SEEK_SET))R fclose(F),0;C*d=(C*)(v+M)-n;J k=fread(d,1,n,F);fclose(F);
 R k==n?mapk_parse(d,n,v,(d-(C*)v)/4):0;}
