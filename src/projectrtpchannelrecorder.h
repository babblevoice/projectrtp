
#ifndef PROJECTRTPCHANNELRECORDER_H
#define PROJECTRTPCHANNELRECORDER_H

#include <memory>
#include <string>

#include <boost/enable_shared_from_this.hpp>

#include "projectrtpsoundfile.h"

class channelrecorder:
  public boost::enable_shared_from_this< channelrecorder >
{
public:
  typedef boost::shared_ptr< channelrecorder > pointer;

  channelrecorder( std::string file, std::function<void( const std::string, const std::string )> );
  ~channelrecorder();

  static pointer create( std::string file, std::function<void( const std::string, const std::string )> );
  uint16_t poweravg( uint16_t power );
  void active( void );
  bool isactive( void ) { return this->_active; }

  std::string file;

  /* In seconds up to MA max size (5 seconds?) */
  uint16_t poweraverageduration;
  /* must have started for this to kick in */
  uint16_t startabovepower;
  /* must have started for this to kick in */
  uint16_t finishbelowpower;
  /* used in conjunction with finishbelowpower */
  uint32_t minduration; /* mSeconds */
  uint32_t maxduration; /* mSeconds */
  int numchannels;
  soundfile::pointer sfile;

  uint16_t lastpowercalc;
  boost::posix_time::ptime created;
  boost::posix_time::ptime activeat;
  std::string finishreason;

  std::function<void( const std::string, const std::string )> f;

private:

  /* Rolling average of power reads */
  ma_filter powerfilter;
  bool _active;

};

#endif /* PROJECTRTPCHANNELRECORDER_H */
