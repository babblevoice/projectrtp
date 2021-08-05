

#include "projectrtpchannelrecorder.h"

/*
# channelrecorder
c'stor and create

Track files we are recording to.
*/
channelrecorder::channelrecorder( std::string &file ) :
  file( file ),
  poweraverageduration( 1 ),
  startabovepower( 0 ),
  finishbelowpower( 0 ),
  minduration( 0 ),
  maxduration( 0 ),
  numchannels( 2 ),
  active( false ),
  lastpowercalc( 0 ),
  created( boost::posix_time::microsec_clock::local_time() )
  //control( nullptr )
{
}

channelrecorder::~channelrecorder()
{
#warning TODO
#if 0

  if( nullptr != this->control )
  {
    JSON::Object v;
    v[ "action" ] = "record";
    v[ "uuid" ] = this->uuid;
    v[ "state" ] = "finished";
    v[ "reason" ] = finishreason;

    this->control->sendmessage( v );
  }
#endif
}

uint16_t channelrecorder::poweravg( uint16_t power )
{
  if( this->poweraverageduration != this->powerfilter.getlength() )
  {
    this->powerfilter.reset( this->poweraverageduration );
  }
  this->lastpowercalc = this->powerfilter.execute( power );
  return this->lastpowercalc;
}

channelrecorder::pointer channelrecorder::create( std::string &file )
{
  return pointer( new channelrecorder( file ) );
}
