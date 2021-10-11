

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
  u_char round;
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
  u_char round;
  int l;
  int32_t rtotal;
  float history[ ma_length ];
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
