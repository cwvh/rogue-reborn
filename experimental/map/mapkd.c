// mapkd.c: reads a .MAP with the schemas of mapk.h written out as code, an op a macro
#include"mapk.h"
#include<setjmp.h>
#include<stdio.h>
#include<string.h>
typedef uint32_t U;typedef uint8_t C;typedef int64_t J;
#define DO(n,...) {J i=0,_n=(n);for(;i<_n;++i){__VA_ARGS__;}}

// the file p..e, the map w[0..m) with o used, the last string's length l
static C*p,*e;static U*w,o,m,l;static jmp_buf jb;
static void X(J b){if(b)longjmp(jb,1);}
static C*t(J n){X(n>e-p);C*r=p;p+=n;return r;}
static U y(){return*t(1);}
static U u(){U r;memcpy(&r,t(4),4);return r;}
static U a(J n){X(n>m-o);U r=o;o+=n;return r;}
static U s(){U n=u();X(!n);C*q=t(n);U k=(n+3)/4,r=a(k);w[r+k-1]=0;memcpy(w+r,q,l=n-1);return r;}
static int v8(C*s){for(int n;*s;){C h=*s++;n=h<128?0:(h&224)==192?1:(h&240)==224?2:(h&248)==240?3:-1;
 if(n<0||n==1&&h<194||n==3&&h>244)return 0;U c=h&63>>n;DO(n,if((*s&192)!=128)return 0;c=c<<6|*s++&63)
 if(n==2&&(c<2048||c-55296<2048)||n==3&&(c<65536||c>1114111))return 0;}return 1;}
static U z(){U r=s();X(!v8((C*)(w+r)));return r;}
static void h(U*q){q[0]=u();q[1]=0;q[2]=s();if(l==7&&!memcmp(w+q[2],"Version",7))q[1]=u(),q[2]=s();}
static void g(const char*x){U n=u();X(!n);C*q=t(n);X(n-1!=strlen(x)||memcmp(q,x,n-1));}

// op  u f   b  s     z  S  R  n  k#    k%    $  @X    a  e  w  !  ?  d
//     F(k)  B  T(k)  Z  S  R  N  W(k)  H(k)  L  A(X)  M  E  Q  O  K  D
#define F(k) memcpy(q,t(4*(k)),4*(k)),q+=(k)
#define B *q++=y()
#define T(k) DO(k,*q++=s())
#define Z *q++=z()
#define S *q++=u(),h(q),q+=3
#define R *q++=0,h(q),q+=3
#define N *q++=c=u()
#define W(k) {J n=(J)c*(k);U i0=*q++=a(n);memcpy(w+i0,t(4*n),4*n);}
#define H(k) {J n=(J)c*(k)*2,k4=(n+3)/4;U i0=*q++=a(k4);if(k4)w[i0+k4-1]=0;memcpy(w+i0,t(n),n);}
#define L {U i0=*q++=a(c);DO(c,w[i0+i]=s())}
#define A(x) {U n=c,i0=*q++=a((J)n*w##x);DO(n,r##x(w+i0+i*w##x))}
#define M {U k=*q++=u();X(k>3||k==2);}
#define E {U k=*q++=y();if(k)X(k>1),*q++=-1;else X((*q++=y())>1);}
#define Q {U k=q0[2]>1?y():-1;*q++=k;memset(q,0,24);if(k+1&&k)memcpy(q,t(24),24);q+=6;}
#define O X(*q++=y())
#define K *q++=rk(q0[1])
#define D {U k=*q++=u(),i0;if(k){i0=a((J)k*wK);DO(k,rK(w+i0+i*wK))}else rF(w+(i0=a(wF)));*q++=i0;}
#define _(x,...) static void r##x(U*q){U*q0=q,c=0;(void)q0;(void)c;__VA_ARGS__;}
#define CM F(12);T(1);F(1);T(11)

_(K,T(1);F(11))
_(F,N;L;F(4))
_(H,T(1);F(8))
_(14,CM;D)
_(15,CM;F(1);N;L;F(3);F(1);T(3);F(5))
_(16,CM;F(1);N;L;F(3);N;L;T(3);F(5))
_(20,Z)
_(25,T(2);F(6);N;L)
_(31,N;A(H))
_(36,)
static U rk(U k){U i0=0;switch(k){
#define C_(n) case n:r##n(w+(i0=a(w##n)));break;
 C_(14)C_(15)C_(16)C_(20)C_(25)C_(31)C_(36)default:X(1);}return i0;}

_(M,S;T(1);F(2);M;F(13);B)
_(D,F(1);N;W(4);H(3);H(3);N;W(3);W(2);W(4))
_(G,Z;F(1);N;H(1))
_(O,S;S;N;W(3);N;A(D);N;W(3);N;W(4);N;H(8);N;A(G))
_(P,S;N;W(3);F(2))
_(Y,S;T(1);F(12);K)
_(V,T(1);N;W(18);N;W(1);O)
_(Q,R;E;B;Q;N;A(V);N;W(2);N;W(1))
_(T,T(1);F(6))
_(L,F(2);N;L)
_(MAP,g("BeginMapv2.1");F(1);S;N;A(M);S;N;A(O);S;N;A(P);S;F(1);S;N;A(Y);S;N;A(Q);S;N;A(T);S;N;A(L);g("EndMap"))

U mapk_parse(const void*d,ptrdiff_t n,U*v,ptrdiff_t M_){p=(C*)d;e=p+n;w=v;o=0;m=M_<0?0:M_>UINT32_MAX?UINT32_MAX:M_;
 if(setjmp(jb))return 0;rMAP(w+a(wMAP));return o;}
U mapk_read(const char*f,U*v,ptrdiff_t M_){FILE*F_=fopen(f,"rb");if(!F_)return 0;J n=fseek(F_,0,SEEK_END)?-1:ftell(F_);
 if(n<0||n>4*M_||fseek(F_,0,SEEK_SET))return fclose(F_),0;C*d=(C*)(v+M_)-n;J k=fread(d,1,n,F_);fclose(F_);
 return k==n?mapk_parse(d,n,v,(d-(C*)v)/4):0;}
