

#ifndef PROJECTRTPFIRFILTER_H
#define PROJECTRTPFIRFILTER_H

#include <stdint.h>

/*!md
# lowpass3_4k16k
Fixed FIR filter. everything fixed for speed.
16K sampling, 3.4K low pass.
*/
#define lowpass3_4k16kfl 17
class lowpass3_4k16k {
public:
  lowpass3_4k16k();
  void reset( void );
  int16_t execute( int16_t val );

private:
  uint8_t round;
  float history[ lowpass3_4k16kfl ];
};

/* Moving Average filter */
/* Based on 20mS history - 50 = 1S - */
#define ma_length (50*5)
class ma_filter {
public:
  ma_filter();
  void reset( int packets );
  int16_t execute( int16_t val );
  inline int getlength( void ) { return this->l; } /* seconds */
  inline int get( void ) { return this->rtotal / this->l; }

private:
  uint8_t round;
  int l;
  int32_t rtotal;
  float history[ ma_length ];
};


/* 
DC filter - thank you https://www.dsprelated.com/freebooks/filters/DC_Blocker.html

x = current input
y = current output
xm1 and ym1 are delayed by 1 samples and start from zero
y = x - xm1 + 0.995 * ym1;
xm1 = x;
ym1 = y;
*/
class dcfilter {
public:
  dcfilter();
  void reset();
  inline int16_t execute( int16_t x ) {
    int16_t y = x - this->xm + 0.995 * this->ym;
    this->xm = x;
    this->ym = y;

    return y;
  }

private:
  int16_t xm;
  int16_t ym;
};

#ifdef TESTSUITE
void testlowpass( void );
void testma( void );
#endif

#ifdef NODE_MODULE
#include <node_api.h>
void initfilter( napi_env env, napi_value &result );
#endif



#endif /* PROJECTRTPFIRFILTER_H */
